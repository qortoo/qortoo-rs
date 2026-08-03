use std::{
    cell::RefCell,
    fmt::Debug,
    io::{IsTerminal, Write},
    sync::OnceLock,
    thread,
};

use itoa::Buffer;
use time::{OffsetDateTime, UtcOffset};
use tracing::{
    Event, Level, Metadata, Subscriber,
    field::{Field, Visit},
    level_filters::LevelFilter,
    span::{Attributes, Id},
};
use tracing_subscriber::{Layer, layer::Context, registry::LookupSpan};

const MESSAGE_FIELD: &str = "message";
const COLLECTION_FIELD: &str = "collection";
const CLIENT_FIELD: &str = "client";
const CUID_FIELD: &str = "cuid";
const DATATYPE_FIELD: &str = "data_key";
const DUID_FIELD: &str = "duid";

#[derive(Default, Debug)]
struct LogContextVisitor {
    msg: Vec<u8>,
    collection: Vec<u8>,
    client: Vec<u8>,
    cuid: Vec<u8>,
    datatype: Vec<u8>,
    duid: Vec<u8>,
}

impl LogContextVisitor {
    fn new() -> Self {
        Self::default()
    }

    #[inline]
    fn message_into(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(self.msg.as_ref());
        write!(buf, "\t\t").unwrap();
    }

    #[inline]
    fn category_into(&self, buf: &mut Vec<u8>) {
        if !self.collection.is_empty() {
            write!(buf, "🗄").unwrap();
            buf.extend_from_slice(self.collection.as_ref());
        }
        if !self.client.is_empty() || !self.cuid.is_empty() {
            write!(buf, "👥").unwrap();
            buf.extend_from_slice(self.client.as_ref());
            write!(buf, "(").unwrap();
            buf.extend_from_slice(self.cuid.as_ref());
            write!(buf, ")").unwrap();
        }
        if !self.datatype.is_empty() || !self.duid.is_empty() {
            write!(buf, "🗂").unwrap();
            buf.extend_from_slice(self.datatype.as_ref());
            write!(buf, "(").unwrap();
            buf.extend_from_slice(self.duid.as_ref());
            write!(buf, ")").unwrap();
        }
        write!(buf, "\t").unwrap();
    }

    /// Fills any empty context fields from `other`, walking from the innermost span outward.
    ///
    /// Returns `true` to signal the caller to keep traversing ancestor spans (at least one
    /// field is still missing), or `false` once all five context fields are collected and
    /// further traversal would yield nothing new.
    fn merge(&mut self, other: &Self) -> bool {
        if !self.collection.is_empty()
            && !self.client.is_empty()
            && !self.cuid.is_empty()
            && !self.datatype.is_empty()
            && !self.duid.is_empty()
        {
            return false;
        }

        if self.collection.is_empty() && !other.collection.is_empty() {
            self.collection = other.collection.clone();
        }
        if self.client.is_empty() && !other.client.is_empty() {
            self.client = other.client.clone();
        }
        if self.cuid.is_empty() && !other.cuid.is_empty() {
            self.cuid = other.cuid.clone();
        }
        if self.datatype.is_empty() && !other.datatype.is_empty() {
            self.datatype = other.datatype.clone();
        }
        if self.duid.is_empty() && !other.duid.is_empty() {
            self.duid = other.duid.clone();
        }
        true
    }
}

impl Visit for LogContextVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            MESSAGE_FIELD => self.msg.extend_from_slice(value.as_bytes()),
            COLLECTION_FIELD => self.collection.extend_from_slice(value.as_bytes()),
            CLIENT_FIELD => self.client.extend_from_slice(value.as_bytes()),
            CUID_FIELD => self.cuid.extend_from_slice(value.as_bytes()),
            DATATYPE_FIELD => self.datatype.extend_from_slice(value.as_bytes()),
            DUID_FIELD => self.duid.extend_from_slice(value.as_bytes()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        let _ = match field.name() {
            MESSAGE_FIELD => write!(self.msg, "{:?}", value),
            COLLECTION_FIELD => write!(self.collection, "{:?}", value),
            CLIENT_FIELD => write!(self.client, "{:?}", value),
            CUID_FIELD => write!(self.cuid, "{:?}", value),
            DATATYPE_FIELD => write!(self.datatype, "{:?}", value),
            DUID_FIELD => write!(self.duid, "{:?}", value),
            _ => Ok(()),
        };
    }
}

pub struct QortooLogLayer {
    pub level_filter: Option<LevelFilter>,
}

impl QortooLogLayer {
    #[inline]
    fn level_str_into(level: &Level, buf: &mut Vec<u8>) {
        static STDOUT_IS_TERMINAL: OnceLock<bool> = OnceLock::new();
        let ansi = *STDOUT_IS_TERMINAL.get_or_init(|| std::io::stdout().is_terminal());
        Self::level_str_with_ansi_into(level, ansi, buf);
    }

    #[inline]
    fn level_str_with_ansi_into(level: &Level, ansi: bool, buf: &mut Vec<u8>) {
        let level = match (*level, ansi) {
            (Level::TRACE, true) => b"\x1b[35m[T] \x1b[0m".as_slice(),
            (Level::DEBUG, true) => b"\x1b[34m[D] \x1b[0m".as_slice(),
            (Level::INFO, true) => b"\x1b[32m[I] \x1b[0m".as_slice(),
            (Level::WARN, true) => b"\x1b[33m[W] \x1b[0m".as_slice(),
            (Level::ERROR, true) => b"\x1b[31m[E] \x1b[0m".as_slice(),
            (Level::TRACE, false) => b"[T] ".as_slice(),
            (Level::DEBUG, false) => b"[D] ".as_slice(),
            (Level::INFO, false) => b"[I] ".as_slice(),
            (Level::WARN, false) => b"[W] ".as_slice(),
            (Level::ERROR, false) => b"[E] ".as_slice(),
        };
        buf.extend_from_slice(level);
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

    fn process_context<S>(ctx: Context<'_, S>, current_visitor: &mut LogContextVisitor)
    where
        S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        if let Some(span) = ctx.lookup_current() {
            for span in span.scope() {
                if let Some(visitor) = span.extensions().get::<LogContextVisitor>() {
                    if !current_visitor.merge(visitor) {
                        return;
                    }
                }
            }
        }
    }
}

impl<S> Layer<S> for QortooLogLayer
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
{
    fn enabled(&self, metadata: &Metadata<'_>, _ctx: Context<'_, S>) -> bool {
        self.level_filter
            .as_ref()
            .map(|level_filter| metadata.level() <= level_filter)
            .unwrap_or(true)
    }

    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            let mut v = LogContextVisitor::new();
            attrs.record(&mut v);
            span.extensions_mut().insert(v);
        }
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

            let mut visitor = LogContextVisitor::new();
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

#[cfg(test)]
mod tests_log_layer {
    use super::*;

    #[test]
    fn can_color_levels_for_a_terminal() {
        let mut buf = Vec::new();
        QortooLogLayer::level_str_with_ansi_into(&Level::INFO, true, &mut buf);
        assert_eq!(buf, b"\x1b[32m[I] \x1b[0m");
    }

    #[test]
    fn can_leave_levels_uncolored_for_a_non_terminal_writer() {
        let mut buf = Vec::new();
        QortooLogLayer::level_str_with_ansi_into(&Level::INFO, false, &mut buf);
        assert_eq!(buf, b"[I] ");
    }

    #[test]
    fn can_merge_missing_context_without_overwriting_inner_values() {
        let mut inner = LogContextVisitor {
            client: b"inner-client".to_vec(),
            datatype: b"counter".to_vec(),
            ..Default::default()
        };
        let outer = LogContextVisitor {
            collection: b"collection".to_vec(),
            client: b"outer-client".to_vec(),
            cuid: b"cuid".to_vec(),
            duid: b"duid".to_vec(),
            ..Default::default()
        };

        assert!(inner.merge(&outer));
        assert_eq!(inner.collection, b"collection");
        assert_eq!(inner.client, b"inner-client");
        assert_eq!(inner.cuid, b"cuid");
        assert_eq!(inner.datatype, b"counter");
        assert_eq!(inner.duid, b"duid");
        assert!(!inner.merge(&LogContextVisitor::default()));
    }

    #[test]
    fn can_format_the_merged_qortoo_context() {
        let visitor = LogContextVisitor {
            collection: b"collection".to_vec(),
            client: b"client".to_vec(),
            cuid: b"cuid".to_vec(),
            datatype: b"counter".to_vec(),
            duid: b"duid".to_vec(),
            ..Default::default()
        };
        let mut buf = Vec::new();

        visitor.category_into(&mut buf);

        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "🗄collection👥client(cuid)🗂counter(duid)\t"
        );
    }
}
