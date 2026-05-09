# Observability

## Overview

Qortoo-rs exposes instrumentation, but applications own exporter setup. The crate does not install a global `tracing` subscriber and no longer has a `tracing` feature.

| Surface | Mechanism | Code |
|---------|-----------|------|
| Trace | `tracing` spans/events plus application-owned OpenTelemetry export | `src/observability/trace.rs`, `examples/observability/trace.rs` |
| Log | Optional Qortoo stdout `tracing_subscriber` layer | `src/observability/log_layer.rs`, `src/observability/trace_context.rs`, `examples/observability/log.rs` |
| Metrics | `metrics` facade calls; application-owned recorder/exporter | `src/observability/metrics.rs`, `examples/observability/metrics.rs` |
| Profile | Application-owned Pyroscope CPU profiler | `examples/observability/profile.rs` |

The local stack in `qortoo-rs-docker/docker-compose.yml` starts Grafana, Prometheus, Tempo, Loki, and Pyroscope:

```shell
make obs-up
# Grafana: http://localhost:3000
# user/password: admin/qortooAdmin
```

The examples live under `examples/observability/`, but `Cargo.toml` registers explicit example targets, so the public commands stay short:

```shell
cargo run --example trace
cargo run --example log
cargo run --example metrics
cargo run --example profile
```

---

## Trace

Qortoo always emits `tracing` spans/events at instrumentation points. To export them, the application installs a subscriber and OpenTelemetry layer.

`examples/observability/trace.rs` configures:

- `tracing_subscriber::Registry` with `EnvFilter` (`qortoo=trace`, `trace=trace`)
- `tracing_subscriber::fmt::layer()` for stdout log output
- `tracing_opentelemetry` layer wired to the OTel tracer
- `opentelemetry_otlp` gRPC exporter (`SpanExporter` via tonic)
- `opentelemetry_sdk::trace::SdkTracerProvider` with batch export
- service name `qortoo-example-trace` via `Resource::builder().with_attribute(KeyValue::new("service.name", ...))`
- `OtelGuard` RAII struct — calls `provider.shutdown()` on drop to flush the batch exporter

Run locally:

```shell
make obs-up
cargo run --example trace
# Grafana -> Explore -> Tempo -> Search -> Service name: qortoo-example-trace
```

Override the OTLP endpoint with standard OpenTelemetry variables:

```shell
# traces-specific variable (checked first)
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://my-collector:4317 cargo run --example trace
# generic fallback
OTEL_EXPORTER_OTLP_ENDPOINT=http://my-collector:4317 cargo run --example trace
```

### Instrumented Spans

| Span | Trigger |
|------|---------|
| `datatype_event_loop` | Event loop lifetime per datatype |
| `push_pull` | Each push/pull sync cycle |
| `execute_local_operation` | Each local write |
| `create_push_pull_pack` | Assembly of outgoing `PushPullPack` |

Span events added with `add_span_event!` annotate moments such as event loop start, push pack send, pull pack receive, and event loop shutdown.

### Span Fields

Datatype spans use these fields:

| Field | Meaning |
|-------|---------|
| `collection` | Collection name |
| `client` | Client alias |
| `cuid` | Client unique ID |
| `data_key` | Datatype key |
| `duid` | Datatype unique ID |

---

## Log

The `log_layer` feature exposes `QortooLogLayer`, Qortoo's compact stdout formatter for `tracing_subscriber`.

```toml
qortoo = { version = "...", features = ["log_layer"] }
```

Create the layer directly — the only field is an optional level filter:

```rust
let fmt = qortoo::QortooLogLayer { level_filter: None };
```

Run with the Qortoo formatter and Loki shipping:

```shell
make obs-up
RUST_LOG=info cargo run --example log --features log_layer
# Grafana -> Explore -> Loki -> {app="qortoo", source="example"}
```

Run with the standard fmt layer (no feature flag needed):

```shell
make obs-up
RUST_LOG=info cargo run --example log
```

`QortooLogLayer` is only a layer. Applications still own the subscriber, `EnvFilter`, and any remote log exporter. The log example builds a `Registry` that chains `EnvFilter → QortooLogLayer (or fmt) → tracing_loki`, shipping logs to `http://localhost:3100` with labels `{app="qortoo", source="example"}`.

The formatter reads the same context fields used by tracing:

```text
collection
client
cuid
data_key
duid
```

### Shipping Test Logs to Loki

The test subscriber (`src/observability/test_subscriber.rs`) is installed once per process via `#[ctor]`. By default it uses the OpenTelemetry subscriber. Setting `QORTOO_RS_LOKI_URL` switches it to ship logs to Loki instead:

```shell
make obs-up
QORTOO_RS_LOKI_URL=http://localhost:3100 RUST_LOG=debug cargo test
# Grafana -> Explore -> Loki -> {app="qortoo", source="test"}
```

Labels attached to every test log line:

| Label | Value |
|-------|-------|
| `app` | `qortoo` |
| `source` | `test` |

If the URL is missing or invalid, the subscriber silently falls back to the OTel backend.

---

## Metrics

Qortoo emits metrics through the `metrics` crate. The application chooses and installs the global recorder.

```
Qortoo-rs
  -> metrics::counter! / metrics::histogram!
  -> application recorder
  -> Prometheus, StatsD, debugging recorder, or another backend
```

### Metric Catalogue

#### `qortoo_sync_total`

Counter incremented for every push/pull sync cycle.

| Label | Values |
|-------|--------|
| `collection` | Collection name |
| `key` | Datatype key |
| `type` | CRDT type, currently `Counter` |
| `result` | `success` or `failure` |

#### `qortoo_sync_duration_seconds`

Histogram for end-to-end `push_pull()` latency in seconds.

| Label | Values |
|-------|--------|
| `collection` | Collection name |
| `key` | Datatype key |
| `type` | CRDT type |

#### `qortoo_backoff_total`

Counter incremented when a recoverable connectivity failure puts the event loop into `BackOff`.

| Label | Values |
|-------|--------|
| `collection` | Collection name |
| `key` | Datatype key |
| `type` | CRDT type |

### Running Locally

`examples/observability/metrics.rs` installs `metrics-exporter-prometheus` and exposes:

```text
http://localhost:9000/metrics
```

Run:

```shell
make obs-up
cargo run --example metrics
# Grafana -> Explore -> Prometheus -> qortoo_sync_total
```

Prometheus is provisioned by `qortoo-rs-docker/prometheus/prometheus.yml` to scrape `host.docker.internal:9000`. On Linux, replace that target with the Docker bridge gateway IP, usually `172.17.0.1:9000`.

### Testing Metrics

Tests use `metrics-util`'s `DebuggingRecorder`.

```toml
metrics-util = { version = "^0.20", features = ["debugging"] }
```

Important behavior in `metrics-util` 0.20: `snapshot()` drains all registered metrics globally. Keep metrics tests serial and take one snapshot per assertion group.

See `src/observability/metrics.rs` and `tests/metrics.rs` for the current test pattern.

---

## Profile

Profiling is application-owned. `examples/observability/profile.rs` uses the Rust Pyroscope client with the pprof backend and exports CPU samples to the Pyroscope service in `qortoo-rs-docker/docker-compose.yml`.

The workload runs two clients against a shared counter — `client-a` writes and syncs, `client-b` subscribes and reads — while a `burn_cpu` function creates measurable CPU samples each iteration. Tags `example=profile` and `library=qortoo` are attached to the Pyroscope application.

Run:

```shell
make obs-up
cargo run --example profile
# Grafana -> Explore -> Pyroscope -> qortoo-example-profile
```

Defaults:

| Setting | Default |
|---------|---------|
| `PYROSCOPE_URL` | `http://localhost:4040` |
| `PYROSCOPE_APPLICATION_NAME` | `qortoo-example-profile` |
| `QORTOO_PROFILE_SECONDS` | `30` |

Override them as needed:

```shell
PYROSCOPE_URL=http://localhost:4040 \
PYROSCOPE_APPLICATION_NAME=qortoo-example-profile \
QORTOO_PROFILE_SECONDS=30 \
cargo run --example profile
```

---

## Local Stack

`make obs-up` starts:

| Service | URL |
|---------|-----|
| Grafana | `http://localhost:3000` |
| Prometheus | `http://localhost:9090` |
| Tempo OTLP gRPC | `http://localhost:4317` |
| Tempo OTLP HTTP | `http://localhost:4318` |
| Loki | `http://localhost:3100` |
| Pyroscope | `http://localhost:4040` |

Shutdown:

```shell
make obs-down
```

Remove persisted volumes too:

```shell
make obs-down-v
```

---

## Quick Reference

| Item | Location | Purpose |
|------|----------|---------|
| `add_span_event!` | `src/observability/trace.rs` | Add an OpenTelemetry span event at the current tracing scope |
| `QortooLogLayer` | `src/observability/log_layer.rs` | Format tracing events to stdout with Qortoo context |
| `QortooTraceContextVisitor` | `src/observability/trace_context.rs` | Collect Qortoo context fields for local formatting |
| `metrics::emit_sync` | `src/observability/metrics.rs` | Emit `qortoo_sync_total` and `qortoo_sync_duration_seconds` |
| `metrics::emit_backoff` | `src/observability/metrics.rs` | Emit `qortoo_backoff_total` |
| Trace example | `examples/observability/trace.rs` | Export traces to Tempo |
| Log example | `examples/observability/log.rs` | Ship logs to Loki |
| Metrics example | `examples/observability/metrics.rs` | Export metrics to Prometheus |
| Profile example | `examples/observability/profile.rs` | Export CPU profiles to Pyroscope |
