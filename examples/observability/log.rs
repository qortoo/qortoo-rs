//! Demonstrates Qortoo's local stdout log layer with Loki shipping.
//!
//! Prerequisites: start the observability stack first.
//!   make obs-up
//!
//! Run with Qortoo's log layer + Loki:
//!   RUST_LOG=info cargo run --example log --features log_layer
//!
//! Run with the standard fmt subscriber + Loki:
//!   RUST_LOG=info cargo run --example log
//!
//! Logs are shipped to http://localhost:3100 (Loki) in the background.
//! View them in Grafana at http://localhost:3000 → Explore → Loki
//! and filter by {app="qortoo", source="example"}.

use qortoo::{Client, Datatype, LocalConnectivity};
use tracing::info;
use url::Url;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_subscriber()?;

    info!("starting qortoo log example");

    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);

    let client = Client::builder("example-log", "client-a")
        .with_connectivity(connectivity)
        .build()?;

    let counter = client.create_datatype("log-counter").build_counter()?;
    counter.increase_by(3)?;
    counter.sync()?;

    info!(value = counter.get_value(), "finished qortoo log example");
    println!("counter value: {}", counter.get_value());

    // Give the Loki background task a moment to flush remaining events.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;

    Ok(())
}

fn setup_subscriber() -> Result<(), Box<dyn std::error::Error>> {
    use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};

    let (loki_layer, loki_task) = tracing_loki::builder()
        .label("app", "qortoo")?
        .label("source", "example")?
        .build_url(Url::parse("http://localhost:3100")?)?;

    tokio::spawn(loki_task);

    #[cfg(feature = "log_layer")]
    let fmt = qortoo::QortooLogLayer { level_filter: None };
    #[cfg(not(feature = "log_layer"))]
    let fmt = tracing_subscriber::fmt::layer();

    let subscriber = Registry::default()
        .with(EnvFilter::from_default_env())
        .with(fmt)
        .with(loki_layer);

    let _ = tracing::subscriber::set_global_default(subscriber);
    Ok(())
}
