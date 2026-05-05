//! Demonstrates CPU profile export to Pyroscope.
//!
//! Prerequisites: start the observability stack first.
//!   make obs-up
//!
//! Run:
//!   cargo run --example profile
//!
//! CPU samples are exported to Pyroscope at http://localhost:4040.
//! View them in Grafana:
//!   http://localhost:3000 -> Explore -> Pyroscope -> qortoo-example-profile
//!
//! Override the endpoint/application/duration with:
//!   PYROSCOPE_URL=http://localhost:4040 \
//!   PYROSCOPE_APPLICATION_NAME=qortoo-example-profile \
//!   QORTOO_PROFILE_SECONDS=30 \
//!   cargo run --example profile

use std::time::{Duration, Instant};

use pyroscope::{
    backend::{BackendConfig, PprofConfig, pprof_backend},
    pyroscope::PyroscopeAgentBuilder,
};
use qortoo::{Client, Counter, Datatype, LocalConnectivity};

const DEFAULT_PYROSCOPE_URL: &str = "http://localhost:4040";
const DEFAULT_APPLICATION_NAME: &str = "qortoo-example-profile";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pyroscope_url =
        std::env::var("PYROSCOPE_URL").unwrap_or_else(|_| DEFAULT_PYROSCOPE_URL.to_string());
    let application_name = std::env::var("PYROSCOPE_APPLICATION_NAME")
        .unwrap_or_else(|_| DEFAULT_APPLICATION_NAME.to_string());
    let profile_seconds = std::env::var("QORTOO_PROFILE_SECONDS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(30);

    println!("Exporting profiles -> {pyroscope_url}");
    println!("Application        -> {application_name}");
    println!("View in Grafana    -> http://localhost:3000 -> Explore -> Pyroscope");
    println!("Running workload for {profile_seconds}s...");

    let agent = PyroscopeAgentBuilder::new(
        pyroscope_url,
        application_name,
        100,
        "pyroscope-rs",
        env!("CARGO_PKG_VERSION"),
        pprof_backend(PprofConfig::default(), BackendConfig::default()),
    )
    .tags(vec![("example", "profile"), ("library", "qortoo")])
    .build()?;

    let agent = agent.start()?;
    run_counter_workload(Duration::from_secs(profile_seconds))?;
    let agent = agent.stop()?;
    agent.shutdown();

    println!("Profile export finished.");
    Ok(())
}

fn run_counter_workload(duration: Duration) -> Result<(), Box<dyn std::error::Error>> {
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);

    let client_a = Client::builder("example-profile", "client-a")
        .with_connectivity(connectivity.clone())
        .build()?;
    let client_b = Client::builder("example-profile", "client-b")
        .with_connectivity(connectivity)
        .build()?;

    let counter_a = client_a
        .create_datatype("profile-counter")
        .build_counter()?;
    let counter_b = client_b
        .subscribe_datatype("profile-counter")
        .build_counter()?;

    let started_at = Instant::now();
    let mut iteration = 0_u64;

    while started_at.elapsed() < duration {
        iteration += 1;
        write_and_sync(&counter_a, iteration)?;
        counter_b.sync()?;

        if iteration.is_multiple_of(10_000) {
            println!(
                "iteration {iteration:>8}: client-a={}, client-b={}",
                counter_a.get_value(),
                counter_b.get_value()
            );
        }
    }

    println!(
        "final counter values: client-a={}, client-b={}",
        counter_a.get_value(),
        counter_b.get_value()
    );
    Ok(())
}

fn write_and_sync(counter: &Counter, iteration: u64) -> Result<(), Box<dyn std::error::Error>> {
    counter.increase_by((iteration % 13 + 1) as i64)?;
    burn_cpu(iteration);
    counter.sync()?;
    Ok(())
}

fn burn_cpu(seed: u64) -> u64 {
    let mut value = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);

    for round in 0..2_048 {
        value = value.rotate_left(7) ^ round;
        value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    }

    value
}
