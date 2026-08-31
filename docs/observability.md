# Observability

## Overview

Qortoo-rs always emits `tracing` spans/events and records metrics through the `metrics`
facade. Applications can install their own subscriber, recorder, and exporters, or enable
one of the managed observability features and ask Qortoo to install the corresponding
process-global pipeline. Creating a `Client` never installs one implicitly.

| Surface | Mechanism | Code |
|---------|-----------|------|
| Trace | `tracing` spans/events plus application-owned or Qortoo-managed OpenTelemetry export | `src/observability/trace.rs`, `examples/observability/trace.rs` |
| Log | Optional Qortoo stdout `tracing_subscriber` layer | `src/observability/log_layer.rs`, `examples/observability/log.rs` |
| Metrics | `metrics` facade calls plus application-owned or Qortoo-managed recorder/exporter | `src/observability/metrics.rs`, `examples/observability/metrics.rs` |
| Profile | Application-owned Pyroscope CPU profiler | `examples/observability/profile.rs` |

## Feature Selection

| Feature | Managed capability |
|---------|--------------------|
| `observability-log` | Exposes `QortooLogLayer`; does not install a subscriber |
| `observability-trace` | Installs stdout logging and an optional OTLP/gRPC trace exporter |
| `observability-metrics` | Installs an optional metrics recorder and Prometheus HTTP exporter |
| `observability` | Umbrella enabling both managed exporter features |

`observability-trace` includes `observability-log` because its managed tracing subscriber
also handles stdout logs. In Rust unit tests, enabling `observability-trace` installs that
log and trace pipeline automatically. `observability-metrics` does not compile tracing
subscriber or OpenTelemetry dependencies. Choose the narrow feature when only one exporter
is needed; `qortoo-ffi` enables the umbrella because foreign bindings can expose both
exporter options.

## Managed Setup

Applications use the crate-root API; `prometheus.rs` is not a separate public entry point.
The following combined setup requires the `observability` umbrella:

```rust
use std::time::Duration;

use qortoo::{
    LogFormat, MetricsSettings, ObservabilitySettings, TraceSettings,
    init_observability, shutdown_observability,
};

init_observability(ObservabilitySettings {
    service_name: "my-service".to_string(),
    log_format: LogFormat::Text,
    trace: Some(TraceSettings::default()),
    metrics: Some(MetricsSettings {
        listen_addr: "0.0.0.0:9000".parse()?,
    }),
    ..Default::default()
})?;

// Create and use Clients here.

shutdown_observability(Duration::from_secs(5))?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

With a narrow feature, the settings surface contains only the matching exporter field:

```rust
// features = ["observability-trace"]
init_observability(ObservabilitySettings {
    trace: Some(TraceSettings::default()),
    ..Default::default()
})?;
```

```rust
// features = ["observability-metrics"]
init_observability(ObservabilitySettings {
    metrics: Some(MetricsSettings {
        listen_addr: "0.0.0.0:9000".parse()?,
    }),
    ..Default::default()
})?;
```

The internal responsibilities are deliberately separated:

| Module | Responsibility |
|--------|----------------|
| `settings.rs` | Resolve and validate log, OTLP, and Prometheus settings |
| `lifecycle.rs` | Enforce one process-global lifecycle and own the exporter runtime/provider |
| `subscriber.rs` | Build the stdout and OTLP tracing layers and install the global subscriber |
| `prometheus.rs` | Configure histogram buckets, install the global metrics recorder, and start the scrape endpoint |

Initialization flows through one orchestrator:

```text
init_observability
  -> lifecycle::init
       -> create a dedicated Tokio runtime when trace or metrics export needs one
       -> subscriber::install when stdout logging or trace export is enabled
       -> prometheus::install when MetricsSettings is present
       -> retain the runtime and tracer provider until shutdown
```

`prometheus::install` is internal. With `observability-metrics`, provide only `metrics`
in `ObservabilitySettings`; trace and stdout-log fields are not part of that feature's
settings surface.

### Lifecycle and Shutdown

| Operation | Result |
|-----------|--------|
| First initialization | Installs the requested pipelines |
| Second initialization | `ObservabilityError::AlreadyInitialized` |
| Shutdown before initialization | Successful no-op |
| Repeated shutdown | Successful no-op |
| Initialization after shutdown | `ObservabilityError::ShutDown` |
| Initialization after an irreversible partial installation | `ObservabilityError::PartiallyInitialized` |

Shutdown flushes and stops the tracer provider and terminates the dedicated runtime, which
stops the Prometheus HTTP exporter. Rust does not provide a way to remove a process-global
`tracing` subscriber or `metrics` recorder, so shutdown is not a complete uninstall:

- a stdout subscriber remains registered;
- the metrics recorder remains registered, although its HTTP exporter is no longer running;
- initialization is terminal after shutdown.

If subscriber installation succeeds and a later metrics installation fails, the subscriber
cannot be rolled back. Qortoo releases the exporter resources it still owns, marks the
lifecycle as failed, returns the original installation error, and rejects later initialization
with `PartiallyInitialized`.

The local stack in `qortoo-rs-docker/docker-compose.yml` starts Grafana, Prometheus, Tempo, Loki, and Pyroscope:

```shell
make obs-up
# Grafana: http://localhost:3000
# user/password: admin/qortooAdmin
```

The examples live under `examples/observability/`, but `Cargo.toml` registers explicit example targets, so the public commands stay short:

```shell
cargo run --features observability-trace --example trace
cargo run --example log
cargo run --features observability-metrics --example metrics
cargo run --example profile
```

---

## Trace

Qortoo always emits `tracing` spans/events at instrumentation points. The trace example
uses `init_observability` to install the subscriber and OpenTelemetry exporter selected by
the application. Enable `observability-trace` for this managed exporter.

`examples/observability/trace.rs` configures:

- `EnvFilter` directives `qortoo=trace,trace=trace`
- `QortooLogLayer` for compact stdout text
- `tracing_opentelemetry` wired to an OTLP/gRPC exporter
- service name `qortoo-example-trace`
- explicit `shutdown_observability` to flush the batch exporter

Run locally:

```shell
make obs-up
cargo run --features observability-trace --example trace
# Grafana -> Explore -> Tempo -> Search -> Service name: qortoo-example-trace
```

Override the OTLP endpoint with standard OpenTelemetry variables:

```shell
# traces-specific variable (checked first)
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://my-collector:4317 \
  cargo run --features observability-trace --example trace
# generic fallback
OTEL_EXPORTER_OTLP_ENDPOINT=http://my-collector:4317 \
  cargo run --features observability-trace --example trace
```

### Instrumented Spans

| Span | Trigger |
|------|---------|
| `datatype_event_loop` | Event loop lifetime per datatype |
| `push_pull` | Each datatype push/pull sync cycle |
| `LocalConnectivity::push_pull` | Processing a push/pull pack through local connectivity |
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

The `observability-log` feature exposes `QortooLogLayer`, Qortoo's compact stdout formatter for
`tracing_subscriber`. `observability-trace` includes `observability-log` and uses
`QortooLogLayer` for `LogFormat::Text`; the metrics-only feature does not install a
tracing subscriber.

```toml
qortoo = { version = "...", features = ["observability-log"] }
```

Create the layer directly — the only field is an optional level filter:

```rust
let fmt = qortoo::QortooLogLayer { level_filter: None };
```

The layer emits ANSI level colors only when stdout is connected to a terminal. Redirected
output and logs collected through a pipe therefore contain plain `[T]`–`[E]` level markers.

Run with the Qortoo formatter and Loki shipping:

```shell
make obs-up
RUST_LOG=info cargo run --example log --features observability-log
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

### Test Logs and Traces

Plain `cargo test` installs no tracing subscriber, so test logs and traces remain off.
Enable `observability-trace` to install the test subscriber once per test process via
`#[ctor]`. The test setup reuses the same subscriber builder and installer as the production
observability path: `QortooLogLayer` writes to stdout and spans are exported over OTLP using
the agent string (for example, `qortoo-0.1.0-<git hash>`) as the Tempo service name.
`RUST_LOG` controls filtering and defaults to `qortoo=debug` when unset:

```shell
cargo test --features observability-trace
```

Because `--all-features` includes `observability-trace`, it enables the same behavior:

```shell
make obs-up
RUST_LOG=debug cargo test --all-features
```

The test helper does not install a test-only Loki layer. Remote log export remains owned
by the application subscriber, as in the log example above; the SDK-provided test setup
only covers stdout logs and OTLP traces.

---

## Metrics

Qortoo emits metrics through the `metrics` crate. The application can install its own
global recorder or provide `MetricsSettings` to `init_observability` for the bundled
Prometheus endpoint by enabling `observability-metrics`.

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
| `type` | CRDT type: `Counter` or `Variable` |
| `result` | `success` or `failure` |

#### `qortoo_sync_duration_seconds`

Histogram for end-to-end `push_pull()` latency in seconds.

| Label | Values |
|-------|--------|
| `collection` | Collection name |
| `type` | CRDT type |
| `result` | `success` or `failure` |

Prometheus exports this metric as a histogram with bucket upper bounds from 5 µs through
10 s, so its `_bucket` series can be aggregated with `histogram_quantile()` and rendered
as a Grafana heatmap.

#### `qortoo_transactions_total`

Counter incremented by the number of regular transactions sent to or received from the
connectivity layer during push/pull sync. Push transactions are counted when the request
is attempted, including attempts that later fail. Pull transactions are counted after a
response is received, even if applying it later fails. Snapshot transactions are not included.

| Label | Values |
|-------|--------|
| `collection` | Collection name |
| `type` | CRDT type |
| `push_pull` | `push` or `pull` |

#### `qortoo_backoff_total`

Counter incremented when a recoverable connectivity failure puts the event loop into `BackOff`.

| Label | Values |
|-------|--------|
| `collection` | Collection name |
| `type` | CRDT type |

### Running Locally

`examples/observability/metrics.rs` enables `MetricsSettings`; the internal
`prometheus.rs` installer exposes:

```text
http://localhost:9000/metrics
```

Run:

```shell
make obs-up
cargo run --features observability-metrics --example metrics
# Grafana -> Explore -> Prometheus -> qortoo_sync_total
```

Prometheus is provisioned by `qortoo-rs-docker/prometheus/prometheus.yml` to scrape `host.docker.internal:9000`. On Linux, replace that target with the Docker bridge gateway IP, usually `172.17.0.1:9000`.

Developer-local scrape configs go in `qortoo-rs-docker/prometheus/conf.d/` — `*.yml` files there are gitignored and loaded through the `scrape_config_files` glob in `prometheus.yml`, so the base config stays untouched. For host-level metrics, run `node_exporter` on the host and enable the bundled example:

```shell
cd qortoo-rs-docker/prometheus/conf.d
cp node_exporter.yml.example node_exporter.yml
make obs-up   # or restart the prometheus container to pick it up
```

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
| `init_observability` / `shutdown_observability` | `src/observability/lifecycle.rs` | Install and own the process-global pipelines |
| Prometheus installer | `src/observability/prometheus.rs` | Internal recorder and scrape-endpoint setup |
| `metrics::emit_sync` | `src/observability/metrics.rs` | Emit `qortoo_sync_total` and `qortoo_sync_duration_seconds` |
| `metrics::emit_pushed_transactions` / `metrics::emit_pulled_transactions` | `src/observability/metrics.rs` | Emit `qortoo_transactions_total` |
| `metrics::emit_backoff` | `src/observability/metrics.rs` | Emit `qortoo_backoff_total` |
| Trace example | `examples/observability/trace.rs` | Export traces to Tempo |
| Log example | `examples/observability/log.rs` | Ship logs to Loki |
| Metrics example | `examples/observability/metrics.rs` | Export metrics to Prometheus |
| Profile example | `examples/observability/profile.rs` | Export CPU profiles to Pyroscope |
