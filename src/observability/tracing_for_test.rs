use std::{cell::RefCell, fmt::Debug, io::Write, sync::OnceLock, thread};

use itoa::Buffer;
use libc::atexit;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use parking_lot::Mutex;
use time::{OffsetDateTime, UtcOffset, format_description::FormatItem, macros::format_description};
use tracing::{
    Event, Id, Level, Metadata, Subscriber,
    field::{Field, Visit},
    metadata::LevelFilter,
    span::Attributes,
};
use tracing_subscriber::{Layer, Registry, layer::Context, prelude::*, registry::LookupSpan};

use crate::{constants, utils::runtime::get_or_init_runtime_handle};

const MESSAGE_FIELD: &str = "message";
const COLLECTION_FIELD: &str = "syncyam.col";
const CLIENT_FIELD: &str = "syncyam.cl";
const CUID_FIELD: &str = "syncyam.cuid";
const DATATYPE_FIELD: &str = "syncyam.dt";
const DUID_FIELD: &str = "syncyam.duid";

static PROVIDER: OnceLock<Mutex<SdkTracerProvider>> = OnceLock::new();

extern "C" fn shutdown_provider() {
    let provider = PROVIDER.get().unwrap();
    let provider = provider.lock();

    if let Err(e) = provider.shutdown() {
        println!("failed to shutdown provider: {:?}", e);
    }
}

pub fn init(level: LevelFilter) {
    let handle = get_or_init_runtime_handle("observability");

    handle.block_on(async move {
        if constants::is_otel_enabled() {
            println!(
                "Initialize open-telemetry tracing with service '{}' for '{}' level",
                constants::get_agent(),
                level
            );
            let exporter = SpanExporter::builder()
                .with_tonic()
                .with_protocol(Protocol::Grpc)
                .build()
                .expect("failed to create otlp exporter");

            let provider = SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(
                    Resource::builder()
                        .with_service_name(constants::get_agent())
                        .build(),
                )
                .build();

            PROVIDER
                .set(Mutex::new(provider.clone()))
                .expect("failed to set provider");

            unsafe {
                let _ = atexit(shutdown_provider);
            }

            let tracer = provider.tracer(constants::get_agent());
            let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
            let filter = tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("syncyam=trace".parse().unwrap())
                .add_directive("integration=trace".parse().unwrap());

            let subscriber = Registry::default()
                .with(telemetry)
                .with(filter)
                .with(SyncYamTracingLayer { opt: Some(level) });
            tracing::subscriber::set_global_default(subscriber)
                .expect("failed to set global default subscriber");
        } else {
            let subscriber = Registry::default().with(SyncYamTracingLayer { opt: Some(level) });
            tracing::subscriber::set_global_default(subscriber)
                .expect("failed to set global default subscriber");
        }
    });
}

#[derive(Default)]
struct SyncYamVisitor {
    msg: Vec<u8>,
    collection: Option<String>,
    client: Option<String>,
    cuid: Option<String>,
    datatype: Option<String>,
    duid: Option<String>,
}

impl SyncYamVisitor {
    fn new() -> Self {
        Self {
            ..Default::default()
        }
    }

    #[inline]
    fn message_into(&self, buf: &mut Vec<u8>) {
        buf.extend_from_slice(self.msg.as_ref());
    }

    #[inline]
    fn category_into(&self, buf: &mut Vec<u8>) {
        let col = self.collection.as_deref().unwrap_or("");
        let client = self.client.as_deref().unwrap_or("");
        let cuid = self.cuid.as_deref().unwrap_or("");
        let datatype = self.datatype.as_deref().unwrap_or("");
        let duid = self.duid.as_deref().unwrap_or("");

        write!(buf, "\t1:{col}|2:{client}|3:{cuid}|4:{datatype}|5:{duid}\t").unwrap();
    }

    fn merge(&mut self, other: &Self) {
        if self.collection.is_none() {
            self.collection = other.collection.clone();
        }
        if self.client.is_none() {
            self.client = other.client.clone();
        }
        if self.cuid.is_none() {
            self.cuid = other.cuid.clone();
        }
        if self.datatype.is_none() {
            self.datatype = other.datatype.clone();
        }
        if self.duid.is_none() {
            self.duid = other.duid.clone();
        }
    }
}

impl Visit for SyncYamVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            MESSAGE_FIELD => self.msg.extend_from_slice(value.as_bytes()),
            COLLECTION_FIELD => self.collection = Some(value.to_owned()),
            CLIENT_FIELD => self.client = Some(value.to_owned()),
            CUID_FIELD => self.cuid = Some(value.to_owned()),
            DATATYPE_FIELD => self.datatype = Some(value.to_owned()),
            DUID_FIELD => self.duid = Some(value.to_owned()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        let _ = match field.name() {
            MESSAGE_FIELD => write!(self.msg, "{:?}", value),
            _ => Ok(()),
        };
    }
}

struct SyncYamTracingLayer {
    opt: Option<LevelFilter>,
}

impl SyncYamTracingLayer {
    #[inline]
    fn level_str_into(level: &Level, buf: &mut Vec<u8>) {
        buf.extend_from_slice(match *level {
            Level::TRACE => b"\x1b[35m[T]\t\x1b[0m",
            Level::DEBUG => b"\x1b[34m[D]\t\x1b[0m",
            Level::INFO => b"\x1b[32m[I]\t\x1b[0m",
            Level::WARN => b"\x1b[33m[W]\t\x1b[0m",
            Level::ERROR => b"\x1b[31m[E]\t\x1b[0m",
        })
    }

    fn local_offset() -> UtcOffset {
        static LOCAL_OFF: OnceLock<UtcOffset> = OnceLock::new();
        *LOCAL_OFF.get_or_init(|| UtcOffset::current_local_offset().unwrap_or(UtcOffset::UTC))
    }

    #[inline]
    fn ts_into(buf: &mut Vec<u8>) {
        static FORMAT: &[FormatItem<'_>] = format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3]"
        );
        let now = OffsetDateTime::now_utc().to_offset(Self::local_offset());
        now.format_into(buf, &FORMAT).unwrap();
        buf.push(b'\t');
    }

    #[inline]
    fn thread_id_into(buf: &mut Vec<u8>) {
        thread_local! {
            static THREAD_LABEL: RefCell<Vec<u8>> = RefCell::new({
                let dbg = format!("{:?}", thread::current().id()); // 최초 1회만
                let trimmed = dbg.strip_prefix("ThreadId(").and_then(|s| s.strip_suffix(')')).unwrap_or(&dbg);
                let mut v = Vec::with_capacity(trimmed.len() + 4);
                v.extend_from_slice(b"[T#");
                v.extend_from_slice(trimmed.as_bytes());
                v.extend_from_slice(b"]\t");
                v
            });
        }
        THREAD_LABEL.with(|s| buf.extend_from_slice(&s.borrow()));
    }

    #[inline]
    fn metadata_into(metadata: &Metadata<'_>, buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(b"\t\t");
        buffer.extend_from_slice(metadata.file().unwrap_or("unknown").as_bytes());
        buffer.extend_from_slice(b":");
        let mut buf = Buffer::new();
        buffer.extend_from_slice(buf.format(metadata.line().unwrap_or_default()).as_bytes());
    }

    fn process_context<S>(ctx: Context<'_, S>, current_visitor: &mut SyncYamVisitor)
    where
        S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    {
        if let Some(span) = ctx.lookup_current() {
            span.scope().for_each(|span| {
                if let Some(visitor) = span.extensions().get::<SyncYamVisitor>() {
                    current_visitor.merge(visitor);
                }
            });
        }
    }
}

impl<S> Layer<S> for SyncYamTracingLayer
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
        let mut v = SyncYamVisitor::new();
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
            Self::thread_id_into(&mut buffer);

            let mut visitor = SyncYamVisitor::new();
            event.record(&mut visitor);

            visitor.message_into(&mut buffer);

            Self::metadata_into(event.metadata(), &mut buffer);
            Self::process_context(ctx, &mut visitor);

            visitor.category_into(&mut buffer);
            OUT.with(|o| {
                let mut out = o.borrow_mut();
                let _ = out.write_all(&buffer);
                let _ = out.write_all(b"\n");
            });
        });
    }
}

#[cfg(test)]
mod tests_tracing {
    use tracing::{Level, debug, error, info, instrument, span, trace, warn};

    #[derive(Debug)]
    struct SpanType {
        client: String,
        cuid: String,
        datatype: String,
        duid: String,
        collection: String,
    }
    #[test]
    fn can_log_message() {
        let span = span!(Level::INFO, "outmost", collection = "col1");
        let _guard = span.enter();

        trace!("trace log");
        debug!("debug log");
        info!("info log");
        warn!("warn log");
        error!("error log");

        span.in_scope(|| {
            info!("in_scope");
        });

        let st = SpanType {
            collection: "collection".to_string(),
            client: "client".to_string(),
            cuid: "cuid".to_string(),
            datatype: "datatype".to_string(),
            duid: "duid".to_string(),
        };
        do_something_level1("duid1", st);
    }

    #[instrument(name = "level1", skip(_st),
        fields(syncyam.cl =_st.client,
        syncyam.cuid = _st.cuid,
        syncyam.duid = _st.duid,
        syncyam.dt = _st.datatype,
        syncyam.col = _st.collection
        ))]
    fn do_something_level1(duid: &str, _st: SpanType) {
        info!("info do_something_level1");
        debug!("debug do_something_level1");
        do_something_level2();
    }

    fn do_something_level2() {
        let span = span!(Level::INFO, "level2");
        let _guard = span.enter();
        info!("inside do_something_level2");
    }

    #[test]
    fn can_log_with_spans() {
        info!("begin can_log_spans");
        client_level("😘");
        info!("end can_log_spans");
    }
    #[instrument(name = "client1", fields(syncyam.cuid=cuid))]
    fn client_level(cuid: &str) {
        let x = span!(Level::INFO, "client_level");
        let _g = x.enter();
        info!(syncyam.cuid = "🙊", "begin client_level");
        client_level2();
        info!("end client_level");
    }

    fn client_level2() {
        info!("begin client_level2");
        datatype_level();
        info!("end client_level2");
    }

    #[instrument(name = "datatype1", fields(syncyam.dt="🙈"))]
    fn datatype_level() {
        info!("begin datatype_level");
        datatype_level2();
        info!("end datatype_level");
    }

    #[instrument(name = "datatype2", fields(syncyam.dt="😘"))]
    fn datatype_level2() {
        info!("begin datatype_level2");
        info!("end datatype_level2");
    }
}
