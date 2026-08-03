//! Installs the `metrics` recorder behind a Prometheus scrape endpoint.

use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use tokio::runtime::Handle;

use crate::{
    errors::observability::ObservabilityError,
    observability::{metrics::SYNC_DURATION_SECONDS, settings::MetricsSettings},
};

/// Sync duration bucket upper bounds in seconds, covering local/in-memory syncs through
/// slow remote operations.
const SYNC_DURATION_BUCKETS_SECONDS: &[f64] = &[
    0.000_005, 0.000_01, 0.000_05, 0.000_1, 0.000_5, 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 1.0, 5.0,
    10.0,
];

fn prometheus_builder() -> Result<PrometheusBuilder, ObservabilityError> {
    PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full(SYNC_DURATION_SECONDS.to_owned()),
            SYNC_DURATION_BUCKETS_SECONDS,
        )
        .map_err(|e| {
            ObservabilityError::Exporter(format!(
                "failed to configure buckets for {SYNC_DURATION_SECONDS}: {e}"
            ))
        })
}

/// Installs the Prometheus recorder and starts its HTTP listener on `runtime`.
pub(super) fn install(
    settings: &MetricsSettings,
    runtime: &Handle,
) -> Result<(), ObservabilityError> {
    // Both the builder's upkeep task and the listener are spawned onto the current
    // runtime, which is why this enters the observability runtime rather than relying
    // on a client one that may be shut down at any time.
    let _enter = runtime.enter();
    let (recorder, exporter) = prometheus_builder()?
        .with_http_listener(settings.listen_addr)
        .build()
        .map_err(|e| {
            ObservabilityError::Exporter(format!(
                "failed to build the Prometheus exporter on {}: {e}",
                settings.listen_addr
            ))
        })?;
    runtime.spawn(exporter);

    metrics::set_global_recorder(recorder).map_err(|e| ObservabilityError::Recorder(e.to_string()))
}

#[cfg(test)]
mod tests_prometheus {
    use super::*;

    #[test]
    fn can_export_sync_duration_as_a_histogram() {
        let recorder = prometheus_builder().unwrap().build_recorder();
        let handle = recorder.handle();

        metrics::with_local_recorder(&recorder, || {
            metrics::histogram!(
                SYNC_DURATION_SECONDS,
                "collection" => "test",
                "type" => "Counter",
                "result" => "success",
            )
            .record(0.001);
        });

        let rendered = handle.render();
        assert!(rendered.contains("qortoo_sync_duration_seconds_bucket{"));
        assert!(rendered.contains("le=\"0.001\""));
        assert!(!rendered.contains("quantile="));
    }
}
