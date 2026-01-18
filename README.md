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
- **Observability**: Optional OpenTelemetry and Jaeger integration for distributed tracing
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

counter.increase().unwrap();
assert_eq!(counter.get_value(), 1);

// Create a read-only counter for observation
let readonly_counter = client
    .subscribe_datatype("observed-counter")
    .with_readonly()
    .build_counter()
    .unwrap();

// This will fail with FailedToWrite error
assert!(readonly_counter.increase().is_err());
```

## Build and Development Commands

```shell
# Install dependencies (cargo-tarpaulin)
make install

# Run all tests
cargo test

# Run tests with tracing/observability (requires Jaeger)
make enable-jaeger
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
```