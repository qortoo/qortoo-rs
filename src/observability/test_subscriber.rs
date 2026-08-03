//! Installs the production tracing stack for feature-enabled unit tests.

use std::sync::OnceLock;

use libc::atexit;
use opentelemetry_sdk::trace::SdkTracerProvider;

use crate::{
    constants,
    errors::observability::ObservabilityError,
    observability::{
        settings::{self, LogFormat, ObservabilitySettings, TraceSettings},
        subscriber,
    },
    utils::runtime::get_or_init_runtime_handle,
};

const DEFAULT_TEST_LOG_FILTER: &str = "qortoo=debug";

// The provider must outlive the subscriber and be shut down after the test process.
static PROVIDER: OnceLock<SdkTracerProvider> = OnceLock::new();

extern "C" fn shutdown_provider() {
    let Some(provider) = PROVIDER.get() else {
        return;
    };

    if let Err(error) = provider.shutdown() {
        eprintln!("failed to shut down the test tracer provider: {error}");
    }
}

#[ctor::ctor(unsafe)]
fn init() {
    // Panicking from a process constructor can abort the test binary. Report an
    // installation conflict and leave the owning subscriber intact instead.
    if let Err(error) = install() {
        eprintln!("failed to install test observability: {error}");
    }
}

fn install() -> Result<(), ObservabilityError> {
    let runtime = get_or_init_runtime_handle("observability");
    let settings = ObservabilitySettings {
        service_name: constants::get_agent().to_string(),
        log_filter: std::env::var("RUST_LOG")
            .ok()
            .and_then(|filter| settings::resolve_log_filter(Some(filter)).ok())
            .unwrap_or_else(|| DEFAULT_TEST_LOG_FILTER.to_string()),
        log_format: LogFormat::Text,
        trace: Some(TraceSettings::default()),
        ..Default::default()
    };
    let trace = settings
        .trace
        .as_ref()
        .expect("test observability always configures trace export");
    let provider = subscriber::build_tracer_provider(&settings, trace, &runtime)?;

    if let Err(error) = subscriber::install(&settings, Some(&provider)) {
        let _ = provider.shutdown();
        return Err(error);
    }
    let _ = PROVIDER.set(provider);
    unsafe {
        let _ = atexit(shutdown_provider);
    }

    Ok(())
}
