//! Demonstrates Qortoo metrics exported to Prometheus.
//!
//! Prerequisites: start the observability stack first.
//!   make obs-up
//!
//! Run:
//!   cargo run --features observability-metrics --example metrics
//!
//! The example exposes a Prometheus scrape endpoint at http://localhost:9000/metrics.
//! The local Prometheus config scrapes host.docker.internal:9000 every 15s.
//! View metrics in Grafana:
//!   http://localhost:3000 → Explore → Prometheus → metric name e.g. qortoo_sync_total

use std::time::Duration;

use qortoo::{Client, Datatype, MetricsSettings, ObservabilitySettings};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    qortoo::init_observability(ObservabilitySettings {
        metrics: Some(MetricsSettings {
            listen_addr: qortoo::resolve_metrics_listen_addr(None)?,
        }),
        ..Default::default()
    })?;

    println!("Scrape endpoint : http://localhost:9000/metrics");
    println!("Prometheus target: host.docker.internal:9000 (macOS/Windows)");
    println!("                   172.17.0.1:9000           (Linux / docker bridge)");
    println!("Ctrl-C to stop.");

    let client = Client::builder("example-metrics", "client-a").build()?;
    let counter = client.create_datatype("counter").build_counter()?;

    let mut iteration: i64 = 0;
    loop {
        iteration += 1;
        counter.increase_by(iteration)?;
        counter.sync()?;

        println!(
            "iteration {iteration:>4}: counter = {}",
            counter.get_value()
        );
        std::thread::sleep(Duration::from_secs(5));
    }
}
