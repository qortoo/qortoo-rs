//! Builds and installs the `tracing` stack: filter, stdout layer, and, when enabled,
//! OTLP trace export.

use opentelemetry::{KeyValue, trace::TracerProvider};
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use tokio::runtime::Handle;
use tracing_subscriber::{EnvFilter, Registry, fmt, layer::SubscriberExt};

use crate::{
    constants,
    errors::observability::ObservabilityError,
    observability::{
        log_layer::QortooLogLayer,
        settings::{LogFormat, ObservabilitySettings, TraceSettings},
    },
};

/// Creates the OTLP/gRPC tracer provider.
///
/// The tonic channel binds to whichever runtime is current when it is built, so this
/// enters `runtime`: that runtime must outlive every export.
pub fn build_tracer_provider(
    settings: &ObservabilitySettings,
    trace: &TraceSettings,
    runtime: &Handle,
) -> Result<SdkTracerProvider, ObservabilityError> {
    let _enter = runtime.enter();
    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_protocol(Protocol::Grpc)
        .with_endpoint(&trace.endpoint)
        .build()
        .map_err(|e| {
            ObservabilityError::Exporter(format!(
                "failed to build the OTLP exporter for '{}': {e}",
                trace.endpoint
            ))
        })?;

    Ok(SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(build_resource(settings))
        .build())
}

/// Resource attributes shared by every span: the same `service.name` the application
/// uses for its own telemetry, plus which SDK and which binding produced the span.
pub fn build_resource(settings: &ObservabilitySettings) -> Resource {
    let mut builder = Resource::builder()
        .with_service_name(settings.service_name.clone())
        .with_attribute(KeyValue::new("qortoo.sdk.language", "rust"))
        .with_attribute(KeyValue::new("qortoo.sdk.version", constants::SDK_VER));
    if let Some(language) = &settings.binding_language {
        builder = builder.with_attribute(KeyValue::new(
            "qortoo.binding.language",
            language.to_owned(),
        ));
    }
    builder.build()
}

/// Installs the process-global subscriber.
///
/// Fails (rather than silently doing nothing) when another library in the process has
/// already installed one — the application needs to know its filter and exporter
/// settings were not applied.
pub fn install(
    settings: &ObservabilitySettings,
    provider: Option<&SdkTracerProvider>,
) -> Result<(), ObservabilityError> {
    let filter = EnvFilter::try_new(&settings.log_filter).map_err(|e| {
        ObservabilityError::InvalidConfig(format!(
            "invalid log filter '{}': {e}",
            settings.log_filter
        ))
    })?;

    let json_layer =
        (settings.log_format == LogFormat::Json).then(|| fmt::layer().json().flatten_event(true));
    let text_layer =
        (settings.log_format == LogFormat::Text).then_some(QortooLogLayer { level_filter: None });

    let subscriber = Registry::default().with(filter);
    let subscriber =
        subscriber
            .with(provider.map(|p| {
                tracing_opentelemetry::layer().with_tracer(p.tracer(constants::SDK_NAME))
            }));
    let subscriber = subscriber.with(json_layer).with(text_layer);

    tracing::subscriber::set_global_default(subscriber)
        .map_err(|e| ObservabilityError::Subscriber(e.to_string()))
}

#[cfg(test)]
mod tests_subscriber {
    use super::*;

    fn settings(filter: &str) -> ObservabilitySettings {
        ObservabilitySettings {
            service_name: "test".into(),
            binding_language: Some("go".into()),
            log_filter: filter.into(),
            ..Default::default()
        }
    }

    #[test]
    fn can_reject_an_invalid_filter_before_touching_the_global_subscriber() {
        let err = install(&settings("=not a filter="), None).unwrap_err();
        assert!(matches!(err, ObservabilityError::InvalidConfig(_)));
    }

    #[test]
    fn can_describe_the_sdk_in_the_resource() {
        let resource = build_resource(&settings("qortoo=info"));
        let language = resource.get(&"qortoo.binding.language".into());
        assert_eq!(language.map(|v| v.to_string()), Some("go".to_string()));
        assert_eq!(
            resource
                .get(&"qortoo.sdk.language".into())
                .map(|v| v.to_string()),
            Some("rust".to_string())
        );
        // The version describes the core SDK, not whichever crate installed the
        // pipeline — a binding reports its own language separately.
        assert_eq!(
            resource
                .get(&"qortoo.sdk.version".into())
                .map(|v| v.to_string()),
            Some(constants::SDK_VER.to_string())
        );
    }

    #[test]
    fn can_omit_the_binding_language_when_the_core_is_used_directly() {
        let resource = build_resource(&ObservabilitySettings::default());
        assert!(
            resource.get(&"qortoo.binding.language".into()).is_none(),
            "a plain Rust application is not driven by a binding"
        );
    }
}
