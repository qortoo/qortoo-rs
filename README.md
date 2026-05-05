# Qortoo rust SDK

[![codecov](https://codecov.io/gh/qortoo/qortoo-rs/branch/main/graph/badge.svg)](https://codecov.io/gh/qortoo/qortoo-rs)
[![CI](https://github.com/qortoo/qortoo-rs/actions/workflows/build-test-coverage.yml/badge.svg)](https://github.com/qortoo/qortoo-rs/actions/workflows/build-test-coverage.yml)
![GitHub commit activity](https://img.shields.io/github/commit-activity/w/qortoo/qortoo-rs)
![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/qortoo/qortoo-rs/build-test-coverage.yml)

Qortoo is a Rust SDK for conflict-free datatypes with distributed synchronization capabilities.

## Features

- **CRDT Datatypes**: Conflict-free replicated data types (Counter, with more coming)
- **Transaction Support**: Atomic transactions with automatic rollback on failure
- **Read-Only Mode**: Create read-only datatypes for observation without modification
- **Event Loop System**: Priority-based event processing with graceful shutdown
- **Connectivity Abstraction**: Pluggable backends for distributed synchronization
- **Push Buffer Management**: Memory-managed operation buffering with configurable limits
- **Checkpoint Tracking**: Sequence synchronization for distributed state
- **Enhanced Error Handling**: Structured stack traces with typed error codes for better debugging
- **Observability**: `tracing` instrumentation with application-owned logs, traces, metrics, and profiling exporters
- **High Code Coverage**: Enforced 90% minimum coverage with cargo-tarpaulin

## Quick Start

```rust
use qortoo::Client;

// Create a client
let client = Client::builder("my-collection", "my-client").build().unwrap();

// Create a writable counter
let counter = client
    .create_datatype("my-counter")
    .build_counter()
    .unwrap();

counter.increase().unwrap();           // increment by 1
counter.increase_by(5).unwrap();       // increment by delta
assert_eq!(counter.get_value(), 6);

// Atomic transaction — all-or-nothing; rolled back on error
counter.transaction("batch", |c| {
    c.increase_by(10)?;
    c.increase_by(5)?;
    Ok(())
}).unwrap();
assert_eq!(counter.get_value(), 21);

// Create a read-only counter for observation
let readonly_counter = client
    .subscribe_datatype("observed-counter")
    .with_readonly()
    .build_counter()
    .unwrap();

// Write operations fail on read-only datatypes
assert!(readonly_counter.increase().is_err());
```

## Feature Flags

| Flag | Description |
|------|-------------|
| `log_layer` | Exports `QortooLogLayer` — Qortoo's compact stdout formatter for `tracing_subscriber` |

## Observability

Qortoo emits `tracing` spans, metrics, and logs. Applications own the exporter setup; the crate installs nothing globally.

Start the local observability stack (Grafana, Prometheus, Tempo, Loki, Pyroscope):

```shell
make obs-up
# Grafana: http://localhost:3000  (admin / qortooAdmin)
```

Run the bundled examples:

```shell
cargo run --example trace    # OpenTelemetry traces → Tempo
cargo run --example log      # structured logs → Loki  (add --features log_layer for QortooLogLayer)
cargo run --example metrics  # Prometheus metrics scrape endpoint
cargo run --example profile  # pprof CPU profiles → Pyroscope
```

See [`docs/observability.md`](docs/observability.md) for the full reference.

## Build and Development Commands

```shell
# Install dependencies (cargo-tarpaulin)
make install

# Run all tests
cargo test

# Run tests with all feature-gated code enabled
cargo test --all-features

# Run a single test
cargo test test_name

# Run tests in a specific module
cargo test module_name::

# Lint (run before PR)
make lint

# Code coverage (requires 90% minimum)
make tarpaulin

# Generate documentation
make doc

# Observability stack
make obs-up        # start Grafana / Prometheus / Tempo / Loki / Pyroscope
make obs-down      # stop the stack
make obs-down-v    # stop and remove persisted volumes
make obs-logs      # tail container logs
```
