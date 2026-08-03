//! Resolved observability configuration: what the caller asked for, merged with the
//! environment and the defaults, in a form the installers can consume directly.
//!
//! Resolution happens here rather than at the installation site so a bad filter or a
//! bad listen address is reported as a configuration error *before* any process-global
//! state is touched.

#[cfg(feature = "observability-metrics")]
use std::net::{SocketAddr, ToSocketAddrs};

#[cfg(feature = "observability-trace")]
use crate::constants;
use crate::errors::observability::ObservabilityError;

/// Service name reported to the trace backend when the caller does not set one.
#[cfg(feature = "observability-trace")]
pub const DEFAULT_SERVICE_NAME: &str = constants::SDK_NAME;
/// Filter used when neither the settings nor `RUST_LOG` provide one.
#[cfg(feature = "observability-trace")]
pub const DEFAULT_LOG_FILTER: &str = "qortoo=info";
/// OTLP gRPC endpoint used when neither the settings nor the OTEL env vars provide one.
#[cfg(feature = "observability-trace")]
pub const DEFAULT_OTLP_ENDPOINT: &str = "http://localhost:4317";
/// Address of the Prometheus scrape endpoint when the caller does not set one.
#[cfg(feature = "observability-metrics")]
pub const DEFAULT_METRICS_LISTEN_ADDR: &str = "0.0.0.0:9000";

/// Destination of the human/machine-readable log stream on stdout.
#[cfg(feature = "observability-trace")]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LogFormat {
    /// No stdout layer. Combined with no trace export this leaves the global subscriber
    /// untouched, so the process (or another Rust library) keeps owning it.
    #[default]
    Off,
    /// One structured JSON object per event.
    Json,
    /// Qortoo's compact human-readable formatter. ANSI colors are enabled only when
    /// stdout is connected to a terminal.
    Text,
}

/// Trace export configuration (OTLP/gRPC).
#[cfg(feature = "observability-trace")]
#[derive(Clone, Debug)]
pub struct TraceSettings {
    /// Endpoint of the OTLP/gRPC collector.
    pub endpoint: String,
}

#[cfg(feature = "observability-trace")]
impl Default for TraceSettings {
    fn default() -> Self {
        Self {
            endpoint: resolve_otlp_endpoint(None),
        }
    }
}

/// Metrics export configuration (Prometheus scrape endpoint).
#[cfg(feature = "observability-metrics")]
#[derive(Clone, Debug)]
pub struct MetricsSettings {
    /// Address the scrape endpoint listens on.
    pub listen_addr: SocketAddr,
}

/// Everything [`crate::init_observability`] needs, with all fallbacks already applied.
///
/// Build it with [`ObservabilitySettings::default`] and override the exporter settings
/// exposed by the selected feature:
///
/// ```
/// let settings = qortoo::ObservabilitySettings::default();
/// ```
#[cfg_attr(not(feature = "observability-trace"), derive(Default))]
#[derive(Clone, Debug)]
pub struct ObservabilitySettings {
    /// `service.name` reported to the trace backend. Set it to the same value the
    /// application uses for its own telemetry.
    #[cfg(feature = "observability-trace")]
    pub service_name: String,
    /// Language of the binding driving the SDK (`qortoo.binding.language`); `None`
    /// omits the attribute.
    #[cfg(feature = "observability-trace")]
    pub binding_language: Option<String>,
    /// `tracing` filter directives, already validated (see [`resolve_log_filter`]).
    #[cfg(feature = "observability-trace")]
    pub log_filter: String,
    /// Stdout log format.
    #[cfg(feature = "observability-trace")]
    pub log_format: LogFormat,
    /// Trace export; `None` installs no exporter.
    #[cfg(feature = "observability-trace")]
    pub trace: Option<TraceSettings>,
    /// Metrics export; `None` installs no recorder.
    #[cfg(feature = "observability-metrics")]
    pub metrics: Option<MetricsSettings>,
}

#[cfg(feature = "observability-trace")]
impl Default for ObservabilitySettings {
    fn default() -> Self {
        Self {
            #[cfg(feature = "observability-trace")]
            service_name: DEFAULT_SERVICE_NAME.to_string(),
            #[cfg(feature = "observability-trace")]
            binding_language: None,
            // Already validated by construction, so `resolve_log_filter` cannot fail here
            // unless `RUST_LOG` itself is malformed — in which case the default applies.
            #[cfg(feature = "observability-trace")]
            log_filter: resolve_log_filter(None).unwrap_or_else(|_| DEFAULT_LOG_FILTER.to_string()),
            #[cfg(feature = "observability-trace")]
            log_format: LogFormat::Off,
            #[cfg(feature = "observability-trace")]
            trace: None,
            #[cfg(feature = "observability-metrics")]
            metrics: None,
        }
    }
}

#[cfg(feature = "observability-trace")]
impl ObservabilitySettings {
    /// Whether installing this configuration claims the process-global subscriber.
    /// Metrics use a separate global (the `metrics` recorder) and do not count.
    pub fn installs_subscriber(&self) -> bool {
        self.log_format != LogFormat::Off || self.trace.is_some()
    }
}

/// Filter precedence: explicit setting, then `RUST_LOG`, then [`DEFAULT_LOG_FILTER`].
///
/// The result is parsed here so a bad filter is reported as a configuration error,
/// before any global state is touched.
#[cfg(feature = "observability-trace")]
pub fn resolve_log_filter(setting: Option<String>) -> Result<String, ObservabilityError> {
    let filter = setting
        .or_else(|| std::env::var("RUST_LOG").ok())
        .unwrap_or_else(|| DEFAULT_LOG_FILTER.to_string());
    tracing_subscriber::EnvFilter::try_new(&filter).map_err(|e| {
        ObservabilityError::InvalidConfig(format!("invalid log filter '{filter}': {e}"))
    })?;
    Ok(filter)
}

/// Endpoint precedence: explicit setting, then the traces-specific OTEL variable, then
/// the generic one, then [`DEFAULT_OTLP_ENDPOINT`].
#[cfg(feature = "observability-trace")]
pub fn resolve_otlp_endpoint(setting: Option<String>) -> String {
    setting
        .or_else(|| std::env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").ok())
        .or_else(|| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok())
        .unwrap_or_else(|| DEFAULT_OTLP_ENDPOINT.to_string())
}

/// Resolves the Prometheus listen address, accepting `host:port` as well as `ip:port`.
#[cfg(feature = "observability-metrics")]
pub fn resolve_metrics_listen_addr(
    setting: Option<String>,
) -> Result<SocketAddr, ObservabilityError> {
    let raw = setting.unwrap_or_else(|| DEFAULT_METRICS_LISTEN_ADDR.to_string());
    raw.to_socket_addrs()
        .map_err(|e| {
            ObservabilityError::InvalidConfig(format!("invalid metrics_listen_addr '{raw}': {e}"))
        })?
        .next()
        .ok_or_else(|| {
            ObservabilityError::InvalidConfig(format!(
                "metrics_listen_addr '{raw}' resolved to no address"
            ))
        })
}

#[cfg(test)]
mod tests_settings {
    use super::*;

    #[test]
    fn can_prefer_the_explicit_filter_over_the_environment() {
        assert_eq!(
            resolve_log_filter(Some("qortoo=trace".into())).unwrap(),
            "qortoo=trace"
        );
    }

    #[test]
    fn can_reject_an_invalid_filter_at_the_configuration_boundary() {
        assert!(matches!(
            resolve_log_filter(Some("=nonsense=".into())),
            Err(ObservabilityError::InvalidConfig(_))
        ));
    }

    #[test]
    fn can_fall_back_to_the_default_endpoint() {
        // Only meaningful when the OTEL variables are unset, which is the case in CI
        // and in a plain developer shell.
        if std::env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT").is_err()
            && std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_err()
        {
            assert_eq!(resolve_otlp_endpoint(None), DEFAULT_OTLP_ENDPOINT);
        }
        assert_eq!(
            resolve_otlp_endpoint(Some("http://collector:4317".into())),
            "http://collector:4317"
        );
    }

    #[test]
    #[cfg(feature = "observability-metrics")]
    fn can_parse_and_reject_listen_addresses() {
        assert_eq!(
            resolve_metrics_listen_addr(Some("127.0.0.1:9111".into()))
                .unwrap()
                .port(),
            9111
        );
        assert!(resolve_metrics_listen_addr(Some("not-an-address".into())).is_err());
    }

    #[test]
    fn can_leave_the_global_subscriber_alone_by_default() {
        // The default settings install nothing, which is what makes a zeroed
        // configuration safe for a process that already owns its subscriber.
        assert!(!ObservabilitySettings::default().installs_subscriber());
        assert!(
            ObservabilitySettings {
                log_format: LogFormat::Json,
                ..Default::default()
            }
            .installs_subscriber()
        );
    }
}
