use std::sync::OnceLock;

use libc::atexit;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use parking_lot::Mutex;
use tracing::metadata::LevelFilter;
use tracing_subscriber::{Registry, layer::SubscriberExt};

use crate::{
    constants, observability::tracing_layer::QortooTracingLayer,
    utils::runtime::get_or_init_runtime_handle,
};

static PROVIDER: OnceLock<Mutex<SdkTracerProvider>> = OnceLock::new();
static TRACING_INITIALIZED: OnceLock<()> = OnceLock::new();

extern "C" fn shutdown_provider() {
    let Some(provider) = PROVIDER.get() else {
        return;
    };
    let provider = provider.lock();

    if let Err(e) = provider.shutdown() {
        println!("failed to shutdown SDK tracer provider: {:?}", e);
    }
}

pub fn init(level: LevelFilter) {
    // Ensure initialization happens only once across all ctor/test paths.
    TRACING_INITIALIZED.get_or_init(|| {
        init_once(level);
    });
}

fn init_once(level: LevelFilter) {
    let handle = get_or_init_runtime_handle("observability");
    // tonic exporter init requires an active Tokio runtime context
    let _enter = handle.enter();
    if constants::is_otel_enabled() {
        init_otel_subscriber(level);
    } else {
        init_local_subscriber(level);
    }
}

fn init_otel_subscriber(level: LevelFilter) {
    println!(
        "Initialize open-telemetry tracing with service '{}' for '{}' level",
        constants::get_agent(),
        level
    );

    let provider = init_otel_provider();
    let tracer = provider.tracer(constants::get_agent());
    let telemetry = tracing_opentelemetry::layer().with_tracer(tracer);
    let filter = tracing_subscriber::EnvFilter::from_default_env()
        .add_directive("qortoo=trace".parse().unwrap())
        .add_directive("integration=trace".parse().unwrap());

    let subscriber = Registry::default()
        .with(telemetry)
        .with(filter)
        .with(QortooTracingLayer { opt: Some(level) });
    set_global_subscriber(subscriber);
}

fn init_local_subscriber(level: LevelFilter) {
    let subscriber = Registry::default().with(QortooTracingLayer { opt: Some(level) });
    set_global_subscriber(subscriber);
}

fn init_otel_provider() -> SdkTracerProvider {
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

    provider
}

fn set_global_subscriber<S>(subscriber: S)
where
    S: tracing::Subscriber + Send + Sync + 'static,
{
    // A host application may install its own global subscriber before this
    // library ctor runs. In that case, leave the existing subscriber intact.
    let _ = tracing::subscriber::set_global_default(subscriber);
}

#[cfg(test)]
mod tests_subscriber {
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
    #[instrument]
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
        do_something_level1(st);
    }

    #[instrument(name = "level1", skip(_st),
        fields(qortoo.cl =_st.client,
        qortoo.cuid = _st.cuid,
        qortoo.duid = _st.duid,
        qortoo.dt = _st.datatype,
        qortoo.col = _st.collection
        ))]
    fn do_something_level1(_st: SpanType) {
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
    #[instrument]
    fn can_log_with_spans() {
        info!("begin can_log_spans");
        client_level("😘");
        info!("end can_log_spans");
    }

    #[instrument(skip_all, name = "client1", fields(qortoo.cuid=_cuid))]
    fn client_level(_cuid: &str) {
        let x = span!(Level::INFO, "client_level", qortoo.cuid = "1");
        let _g = x.enter();
        info!(qortoo.cuid = "🙊", "begin client_level");
        client_level2();
        info!("end client_level");
    }

    fn client_level2() {
        info!("begin client_level2");
        datatype_level();
        info!("end client_level2");
    }

    #[instrument(name = "datatype1", fields(qortoo.dt="🙈"))]
    fn datatype_level() {
        info!("begin datatype_level");
        datatype_level2();
        info!("end datatype_level");
    }

    #[instrument(name = "datatype2", fields(qortoo.dt="😘"))]
    fn datatype_level2() {
        info!("begin datatype_level2");
        info!("end datatype_level2");
    }
}
