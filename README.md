# SyncYam rust SDK

[![codecov](https://codecov.io/gh/syncyam-io/syncyam-rs/branch/main/graph/badge.svg)](https://codecov.io/gh/syncyam-io/syncyam-rs)
[![CI](https://github.com/syncyam-io/syncyam-rs/actions/workflows/coverage.yml/badge.svg)](https://github.com/syncyam-io/syncyam-rs/actions/workflows/coverage.yml)
![GitHub commit activity](https://img.shields.io/github/commit-activity/w/syncyam-io/syncyam-rs)
![GitHub Actions Workflow Status](https://img.shields.io/github/actions/workflow/status/syncyam-io/syncyam-rs/build-test-coverage.yml)

SyncYam is a Rust SDK for conflict-free datatypes with distributed synchronization capabilities.

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
use syncyam::Client;

// Create a client
let client = Client::builder("my-collection", "my-client").build();

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

## For development

### Getting started

```shell
# install 
$ make install
# 
$ make enable-jeager 
```

> [!NOTE]
> To enable log output in the tests, you should run test with '--all-features' after running the follows:

```shell
$ make enable-jaeger
$ cargo test --all-features 
```

You can find the traces in the jaeger UI: http://localhost:16686/

### Code Coverage

Code coverage is measured using cargo-tarpaulin:

```shell
# run tarpaulin
$ make tarpaulin

# local update coverage badge
$ make update-coverage-badge
```

### Before pull request

Run the full lint suite before submitting:

```shell
$ make lint
```
