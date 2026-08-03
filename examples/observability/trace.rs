//! Demonstrates OpenTelemetry trace export to Tempo through `qortoo::init_observability`.
//!
//! Prerequisites: start the observability stack first.
//!   make obs-up
//!
//! Run:
//!   cargo run --features observability-trace --example trace
//!
//! The SDK installs nothing on its own; this example asks for the trace pipeline
//! explicitly and exports via OTLP gRPC to Tempo (http://localhost:4317).
//! View traces in Grafana:
//!   http://localhost:3000 → Explore → Tempo → Search → Service name: qortoo-example-trace
//!
//! Override the OTLP endpoint via environment variables (standard OpenTelemetry):
//!   OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://my-collector:4317 \
//!     cargo run --features observability-trace --example trace

use std::time::Duration;

use qortoo::{
    Client, Counter, Datatype, LocalConnectivity, LogFormat, ObservabilitySettings, TraceSettings,
};
use tracing::{info, instrument};

const SERVICE_NAME: &str = "qortoo-example-trace";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    setup_tracing()?;

    run_counter_sync()?;

    // Flush and stop the exporter; the timeout bounds how long that may take.
    qortoo::shutdown_observability(Duration::from_secs(5))?;

    Ok(())
}

// Create a root span that wraps the example workload.
#[instrument(
    name = "example.counter_sync",
    fields(collection = "example-trace", clients = 2)
)]
fn run_counter_sync() -> Result<(), Box<dyn std::error::Error>> {
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);

    let client1 = build_client("client-a", connectivity.clone())?;
    let client2 = build_client("client-b", connectivity)?;

    let counter1 = client1.create_datatype("shared-counter").build_counter()?;
    write_and_sync(&counter1, 5)?;

    let counter2 = client2
        .subscribe_datatype("shared-counter")
        .build_counter()?;
    pull_and_read(&counter2)?;

    info!(
        client_a = counter1.get_value(),
        client_b = counter2.get_value(),
        "both clients synced"
    );
    println!(
        "counter values after sync: client-a={}, client-b={}",
        counter1.get_value(),
        counter2.get_value()
    );

    Ok(())
}

#[instrument(skip(connectivity), fields(alias = alias))]
fn build_client(
    alias: &str,
    connectivity: std::sync::Arc<LocalConnectivity>,
) -> Result<qortoo::Client, Box<dyn std::error::Error>> {
    let client = Client::builder("example-trace", alias)
        .with_connectivity(connectivity)
        .build()?;
    info!("client built");
    Ok(client)
}

#[instrument(skip(counter), fields(delta = delta))]
fn write_and_sync(counter: &Counter, delta: i64) -> Result<(), Box<dyn std::error::Error>> {
    counter.increase_by(delta)?;
    info!(value_after_write = counter.get_value(), "increased");
    counter.sync()?;
    info!(value_after_sync = counter.get_value(), "synced");
    Ok(())
}

#[instrument(skip(counter))]
fn pull_and_read(counter: &Counter) -> Result<(), Box<dyn std::error::Error>> {
    counter.sync()?;
    info!(value = counter.get_value(), "pulled");
    Ok(())
}

// --- Application-owned OTel bootstrap ----------------------------------------

/// Asks the SDK to install the trace pipeline. The application still owns every
/// decision — service identity, filter, endpoint — it just no longer assembles the
/// exporter, the resource, and the subscriber by hand.
fn setup_tracing() -> Result<(), Box<dyn std::error::Error>> {
    let trace = TraceSettings::default();
    println!("Exporting traces → {}", trace.endpoint);
    println!("View in Grafana  → http://localhost:3000 → Explore → Tempo");

    qortoo::init_observability(ObservabilitySettings {
        service_name: SERVICE_NAME.to_string(),
        log_filter: "qortoo=trace,trace=trace".to_string(),
        log_format: LogFormat::Text,
        trace: Some(trace),
        ..Default::default()
    })?;
    Ok(())
}
