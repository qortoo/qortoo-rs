//! Demonstrates application-owned OpenTelemetry trace export to Tempo.
//!
//! Prerequisites: start the observability stack first.
//!   make obs-up
//!
//! Run:
//!   cargo run --example trace
//!
//! This example installs its own subscriber and exports traces via OTLP gRPC
//! to Tempo (http://localhost:4317).
//! View traces in Grafana:
//!   http://localhost:3000 → Explore → Tempo → Search → Service name: qortoo-example-trace
//!
//! Override the OTLP endpoint via environment variables (standard OpenTelemetry):
//!   OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://my-collector:4317 cargo run --example trace

use opentelemetry::{KeyValue, trace::TracerProvider};
use opentelemetry_otlp::{Protocol, SpanExporter, WithExportConfig};
use opentelemetry_sdk::{Resource, trace::SdkTracerProvider};
use qortoo::{Client, Counter, Datatype, LocalConnectivity};
use tracing::{info, instrument};
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _guard = setup_tracing()?;

    run_counter_sync()?;

    // Give the batch exporter time to flush before shutdown.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

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

struct OtelGuard(SdkTracerProvider);

impl Drop for OtelGuard {
    fn drop(&mut self) {
        if let Err(e) = self.0.shutdown() {
            eprintln!("OTel provider shutdown error: {e:?}");
        }
    }
}

fn setup_tracing() -> Result<OtelGuard, Box<dyn std::error::Error>> {
    let endpoint = std::env::var("OTEL_EXPORTER_OTLP_TRACES_ENDPOINT")
        .or_else(|_| std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT"))
        .unwrap_or_else(|_| "http://localhost:4317".to_string());

    println!("Exporting traces → {endpoint}");
    println!("View in Grafana  → http://localhost:3000 → Explore → Tempo");

    let exporter = SpanExporter::builder()
        .with_tonic()
        .with_protocol(Protocol::Grpc)
        .with_endpoint(&endpoint)
        .build()?;

    let provider = SdkTracerProvider::builder()
        .with_batch_exporter(exporter)
        .with_resource(
            Resource::builder()
                .with_attribute(KeyValue::new("service.name", "qortoo-example-trace"))
                .build(),
        )
        .build();

    let tracer = provider.tracer("qortoo-example");
    let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);

    let filter = EnvFilter::from_default_env()
        .add_directive("qortoo=trace".parse()?)
        .add_directive("trace=trace".parse()?);

    let subscriber = Registry::default()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(otel_layer);

    tracing::subscriber::set_global_default(subscriber)?;

    Ok(OtelGuard(provider))
}
