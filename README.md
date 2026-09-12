# Qortoo rust SDK

[![codecov](https://codecov.io/gh/qortoo/qortoo-rs/branch/main/graph/badge.svg)](https://codecov.io/gh/qortoo/qortoo-rs)
[![CI](https://github.com/qortoo/qortoo-rs/actions/workflows/build-test-coverage.yml/badge.svg)](https://github.com/qortoo/qortoo-rs/actions/workflows/build-test-coverage.yml)
[![GitHub commit activity](https://img.shields.io/github/commit-activity/w/qortoo/qortoo-rs)](https://github.com/qortoo/qortoo-rs/graphs/commit-activity)
[![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/qortoo/qortoo-rs/build-test-coverage.yml)](https://github.com/qortoo/qortoo-rs/actions/workflows/build-test-coverage.yml)

Qortoo is a Rust SDK for conflict-free replicated datatypes (CRDTs) with atomic
transactions and a pluggable connectivity trait for synchronization. This is a pre-release
crate (`0.1.0`, not yet published to crates.io) consumed by pinning a commit as a git or path
dependency; `qortoo-go`'s CI does exactly this.

## Features

- **CRDT Datatypes**: `Counter` and [`Variable`](docs/variable.md) (last-write-wins), with more planned
- **Transaction Support**: Atomic transactions with automatic rollback on failure
- **Read-Only Mode**: Create read-only datatypes for observation without modification
- **Event Loop System**: Priority-based event processing with graceful shutdown
- **Connectivity Abstraction**: A `Connectivity` trait for synchronization backends. The two
  bundled backends are in-process only — the default backend answers a single client and shares
  nothing, and the local backend simulates a server for clients sharing one process (see
  [`docs/connectivity.md`](docs/connectivity.md)); nothing ships yet for syncing across a network.
- **Push Buffer Management**: Memory-managed operation buffering with configurable limits
- **Checkpoint Tracking**: Sequence synchronization for distributed state
- **Enhanced Error Handling**: Structured stack traces with typed error codes for better debugging
- **Observability**: `tracing` instrumentation with application-owned logs, traces, metrics, and profiling exporters
- **Code Coverage**: CI gates merges at 80% minimum; `make tarpaulin` runs a stricter 90% check locally

## Requirements

- Edition 2024, with a declared minimum of Rust `1.87.0` (`rust-version` in `Cargo.toml`). CI
  builds and tests only against `1.97.1`; earlier toolchains down to the declared minimum are
  not built in CI and are not verified to work.

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
| `observability-log` | Exports `QortooLogLayer` — Qortoo's compact stdout formatter with automatic ANSI terminal detection |
| `observability-trace` | Enables managed stdout logging and OTLP/gRPC trace export, including the unit-test subscriber |
| `observability-metrics` | Enables opt-in process-global metrics export through Prometheus |
| `observability` | Umbrella enabling both managed exporter features |

## Observability

Qortoo emits `tracing` spans, metrics, and logs. The crate installs nothing globally by default; applications can own the exporter setup or opt into only the managed exporter they need. The `observability` umbrella remains available when both are required.

Start the local observability stack (Grafana, Prometheus, Tempo, Loki, Pyroscope):

```shell
make obs-up
# Grafana: http://localhost:3000  (admin / qortooAdmin)
```

Run the bundled examples:

```shell
cargo run --features observability-trace --example trace    # OpenTelemetry traces → Tempo
cargo run --example log      # structured logs → Loki  (add --features observability-log for QortooLogLayer)
cargo run --features observability-metrics --example metrics  # Prometheus metrics scrape endpoint
cargo run --example profile  # pprof CPU profiles → Pyroscope
```

See [`docs/observability.md`](docs/observability.md) for the full reference.

## Documentation

[`docs/README.md`](docs/README.md) indexes every concept document by reading path — first use,
how a datatype works, operating it, and contributing. `cargo doc --no-deps --open` generates the
API reference from source.

## Build and Development Commands

```shell
# Install dependencies (cargo-tarpaulin)
make install

# Run all tests without installing a log/trace subscriber
cargo test

# Run tests with Qortoo logs and OTLP trace export
cargo test --features observability-trace

# Run tests with all feature-gated code enabled
cargo test --all-features

# Run a single test
cargo test test_name

# Run tests in a specific module
cargo test module_name::

# Lint (run before PR)
make lint

# Code coverage (90% minimum locally; CI gates at 80%)
make tarpaulin

# Generate documentation
make doc

# Observability stack
make obs-up        # start Grafana / Prometheus / Tempo / Loki / Pyroscope
make obs-down      # stop the stack
make obs-down-v    # stop and remove persisted volumes
make obs-logs      # tail container logs
```
