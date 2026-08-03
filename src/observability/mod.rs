#[cfg(feature = "observability-log")]
pub mod log_layer;

pub mod metrics;
pub mod trace;

// Opt-in, process-global installation. Each exporter can be compiled independently;
// `observability` is an umbrella over both.
#[cfg(any(feature = "observability-trace", feature = "observability-metrics"))]
pub mod lifecycle;
#[cfg(feature = "observability-metrics")]
mod prometheus;
#[cfg(any(feature = "observability-trace", feature = "observability-metrics"))]
pub mod settings;
#[cfg(feature = "observability-trace")]
pub mod subscriber;

#[cfg(all(test, feature = "observability-trace"))]
pub mod test_subscriber;
