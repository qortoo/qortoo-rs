use std::{cell::RefCell, io::Write, thread};

use itoa::Buffer;
use time::{OffsetDateTime, UtcOffset};
use tracing::{
    Event, Level, Metadata, Subscriber,
    level_filters::LevelFilter,
    span::{Attributes, Id},
};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

use crate::observability::visitor::QortooVisitor;

pub struct QortooTracingLayer {
    pub opt: Option<LevelFilter>,
}

impl QortooTracingLayer {
    #[inline]
    fn level_str_into(level: &Level, buf: &mut Vec<u8>) {
        buf.extend_from_slice(match *level {
            Level::TRACE => b"\x1b[35m[T] \x1b[0m",
            Level::DEBUG => b"\x1b[34m[D] \x1b[0m",
            Level::INFO => b"\x1b[32m[I] \x1b[0m",
            Level::WARN => b"\x1b[33m[W] \x1b[0m",
            Level::ERROR => b"\x1b[31m[E] \x1b[0m",
        })
    }

    #[inline]
    fn ts_into(buf: &mut Vec<u8>) {
        let now = OffsetDateTime::now_utc().to_offset(Self::local_offset());
        now.format_into(buf, &time::format_description::well_known::Rfc2822)
            .unwrap();
        buf.push(b'\t');
    }

    #[inline]
    fn local_offset() -> UtcOffset {
        UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC)
    }

    #[inline]
    fn thread_id_into(buf: &mut Vec<u8>) {
        thread_local! {
            static THREAD_LABEL: RefCell<Vec<u8>> = RefCell::new({
                let dbg = format!("{:?}", thread::current().id());
                let trimmed = dbg.strip_prefix("ThreadId(").and_then(|s| s.strip_suffix(')')).unwrap_or(&dbg);
                let mut v = Vec::with_capacity(trimmed.len() + 4);
                v.extend_from_slice(b"[\xF0\x9F\xA7\xB5#");
                v.extend_from_slice(trimmed.as_bytes());
                v.extend_from_slice(b"]\t");
                v
            });
        }
        THREAD_LABEL.with(|s| buf.extend_from_slice(&s.borrow()));
    }

    #[inline]
    fn metadata_into(metadata: &Metadata<'_>, buffer: &mut Vec<u8>) {
        write!(buffer, "🗂️ ").unwrap();
        buffer.extend_from_slice(metadata.file().unwrap_or("unknown").as_bytes());
        buffer.extend_from_slice(b":");
        let mut buf = Buffer::new();
        buffer.extend_from_slice(buf.format(metadata.line().unwrap_or_default()).as_bytes());
    }

    fn process_context<S>(ctx: Context<'_, S>, current_visitor: &mut QortooVisitor)
    where
        S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        if let Some(span) = ctx.lookup_current() {
            for span in span.scope() {
                if let Some(visitor) = span.extensions().get::<QortooVisitor>() {
                    if !current_visitor.merge(visitor) {
                        return;
                    }
                }
            }
        }
    }
}

impl<S> Layer<S> for QortooTracingLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn enabled(&self, metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        self.opt
            .as_ref()
            .map(|level_filter| metadata.level() <= level_filter)
            .unwrap_or(true)
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let span = ctx.span(id).expect("failed to get span");
        let mut v = QortooVisitor::new();
        attrs.record(&mut v);
        span.extensions_mut().insert(v);
    }

    fn on_event(&self, event: &Event, ctx: Context<'_, S>) {
        thread_local! {
            static BUF: RefCell<Vec<u8>> = RefCell::new(Vec::with_capacity(2048));
            static OUT: RefCell<std::io::LineWriter<std::io::Stdout>> = RefCell::new(std::io::LineWriter::new(std::io::stdout()));
        }

        BUF.with(|b| {
            let mut buffer = b.borrow_mut();
            buffer.clear();

            Self::ts_into(&mut buffer);
            Self::level_str_into(event.metadata().level(), &mut buffer);

            let mut visitor = QortooVisitor::new();
            event.record(&mut visitor);
            visitor.message_into(&mut buffer);

            Self::thread_id_into(&mut buffer);
            Self::process_context(ctx, &mut visitor);
            visitor.category_into(&mut buffer);
            Self::metadata_into(event.metadata(), &mut buffer);

            OUT.with(|o| {
                let mut out = o.borrow_mut();
                let _ = out.write_all(&buffer);
                let _ = out.write_all(b"\n");
            });
        });
    }
}
