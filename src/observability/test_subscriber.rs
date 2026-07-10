use std::sync::OnceLock;

use libc::atexit;
use opentelemetry::trace::TracerProvider;
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{
    Resource,
    trace::{SdkTracer, SdkTracerProvider},
};
use parking_lot::Mutex;
use tracing_opentelemetry::OpenTelemetryLayer;
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};

#[cfg(feature = "log_layer")]
use crate::observability::log_layer::QortooLogLayer;
use crate::{constants, utils::runtime::get_or_init_runtime_handle};

static TRACING_INITIALIZED: OnceLock<()> = OnceLock::new();
static PROVIDER: OnceLock<Mutex<SdkTracerProvider>> = OnceLock::new();

extern "C" fn shutdown_provider() {
    let Some(provider) = PROVIDER.get() else {
        return;
    };
    let provider = provider.lock();

    if let Err(e) = provider.shutdown() {
        println!("failed to shutdown SDK tracer provider: {:?}", e);
    }
}

#[ctor::ctor(unsafe)]
pub fn init() {
    // Test builds install this subscriber through a ctor; do it once per process.
    TRACING_INITIALIZED.get_or_init(|| {
        init_once();
    });
}

fn init_once() {
    let handle = get_or_init_runtime_handle("observability");
    // The tonic OTLP exporter requires an active Tokio runtime context.
    let _enter = handle.enter();
    let env_filter = build_env_filter();
    let loki_url = std::env::var("QORTOO_RS_LOKI_URL").ok();
    if let Some(url) = loki_url {
        init_subscriber_with_loki(env_filter, url, &handle);
    } else {
        init_subscriber(env_filter);
    }
}

fn init_subscriber(env_filter: EnvFilter) {
    let env_filter_display = env_filter.to_string();
    let subscriber = Registry::default()
        .with(build_telemetry_layer())
        .with(env_filter)
        .with(build_fmt_layer());
    if tracing::subscriber::set_global_default(subscriber).is_ok() {
        println!(
            "Initialized OpenTelemetry tracing with service '{}' and filter '{}'",
            constants::get_agent(),
            env_filter_display
        );
    }
}

fn build_telemetry_layer<S>() -> OpenTelemetryLayer<S, SdkTracer>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    let provider = init_provider();
    let tracer = provider.tracer(constants::get_agent());
    tracing_opentelemetry::layer().with_tracer(tracer)
}

#[cfg(feature = "log_layer")]
fn build_fmt_layer() -> QortooLogLayer {
    QortooLogLayer { level_filter: None }
}

#[cfg(not(feature = "log_layer"))]
fn build_fmt_layer<S>() -> impl tracing_subscriber::Layer<S>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    tracing_subscriber::fmt::layer()
}

fn init_subscriber_with_loki(
    env_filter: EnvFilter,
    loki_url: String,
    handle: &tokio::runtime::Handle,
) {
    let Ok(parsed_url) = url::Url::parse(&loki_url) else {
        eprintln!("QORTOO_RS_LOKI_URL is not a valid URL: {loki_url}");
        init_subscriber(env_filter);
        return;
    };

    let builder = tracing_loki::builder()
        .label("app", "qortoo")
        .and_then(|b| b.label("source", "test"))
        .and_then(|b| b.build_url(parsed_url));

    match builder {
        Ok((loki_layer, loki_task)) => {
            handle.spawn(loki_task);

            let env_filter_display = env_filter.to_string();
            let subscriber = Registry::default()
                .with(build_telemetry_layer())
                .with(env_filter)
                .with(build_fmt_layer())
                .with(loki_layer);
            if tracing::subscriber::set_global_default(subscriber).is_ok() {
                println!(
                    "Initialized OpenTelemetry tracing with service '{}', filter '{}', and Loki: {}",
                    constants::get_agent(),
                    env_filter_display,
                    loki_url
                );
            }
        }
        Err(e) => {
            eprintln!("failed to build Loki layer: {e}");
            init_subscriber(env_filter);
        }
    }
}

fn build_env_filter() -> EnvFilter {
    EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::default().add_directive("qortoo=debug".parse().unwrap()))
}

fn init_provider() -> SdkTracerProvider {
    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_protocol(Protocol::Grpc)
        .build()
        .expect("failed to create OTLP exporter");

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(
            Resource::builder()
                .with_service_name(constants::get_agent())
                .build(),
        )
        .build();

    let _ = PROVIDER.set(Mutex::new(provider.clone()));

    unsafe {
        let _ = atexit(shutdown_provider);
    }
    provider
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
        fields(client =_st.client,
        cuid = _st.cuid,
        duid = _st.duid,
        data_key = _st.datatype,
        collection = _st.collection
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

    #[instrument(skip_all, name = "client1", fields(cuid=_cuid))]
    fn client_level(_cuid: &str) {
        let x = span!(Level::INFO, "client_level", cuid = "1");
        let _g = x.enter();
        info!(cuid = "🙊", "begin client_level");
        client_level2();
        info!("end client_level");
    }

    fn client_level2() {
        info!("begin client_level2");
        datatype_level();
        info!("end client_level2");
    }

    #[instrument(name = "datatype1", fields(data_key = "🙈"))]
    fn datatype_level() {
        info!("begin datatype_level");
        datatype_level2();
        info!("end datatype_level");
    }

    #[instrument(name = "datatype2", fields(data_key = "😘"))]
    fn datatype_level2() {
        info!("begin datatype_level2");
        info!("end datatype_level2");
    }
}
