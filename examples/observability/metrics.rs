//! Demonstrates Qortoo metrics exported to Prometheus.
//!
//! Prerequisites: start the observability stack first.
//!   make obs-up
//!
//! Run:
//!   cargo run --example metrics
//!
//! The example exposes a Prometheus scrape endpoint at http://localhost:9000/metrics.
//! The local Prometheus config scrapes host.docker.internal:9000 every 15s.
//! View metrics in Grafana:
//!   http://localhost:3000 → Explore → Prometheus → metric name e.g. qortoo_sync_total

use std::time::Duration;

use metrics_exporter_prometheus::PrometheusBuilder;
use qortoo::{Client, Datatype};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    PrometheusBuilder::new()
        .with_http_listener(([0, 0, 0, 0], 9000))
        .install()?;

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
        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}
