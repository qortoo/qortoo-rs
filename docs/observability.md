# Observability

## Overview

Qortoo-rs provides two complementary observability pillars:

| Pillar | Mechanism | Location |
|--------|-----------|----------|
| **Tracing** | OpenTelemetry spans and events (opt-in via `tracing` feature) | `src/observability/tracing_layer.rs` |
| **Metrics** | `metrics` crate facade — Prometheus-compatible counters and histograms | `src/observability/metrics.rs` |

Both follow a **facade pattern**: the library only calls a thin abstraction layer; the actual backend (Jaeger, Prometheus, etc.) is registered by the application at startup. No backend = zero runtime cost.

---

## Tracing

### Enabling

Tracing is disabled by default. Enable it with the `tracing` feature:

```toml
[dependencies]
qortoo = { version = "...", features = ["tracing"] }
```

This pulls in `opentelemetry_sdk`, `opentelemetry-otlp`, and `tracing-subscriber`. Without the feature, the `tracing` facade calls compile to no-ops.

### What is instrumented

| Span | Trigger |
|------|---------|
| `datatype_event_loop` | Event loop lifetime per datatype |
| `push_pull` | Each push/pull sync cycle |
| `execute_local_operation` | Each local write |
| `create_push_pull_pack` | Assembly of outgoing `PushPullPack` |

Span events (via `add_span_event!`) annotate key moments within a span:

```
datatype_event_loop
  ├── "start event_loop"
  ├── "send PUSH PushPullPack"   (ppp = <serialized pack>)
  ├── "recv PULL PushPullPack"
  └── "quiting event_loop"
```

### Span fields

Every datatype span carries these fields for filtering in Jaeger/Tempo:

```
qortoo.col   — collection name
qortoo.cl    — client alias
qortoo.cuid  — client unique ID
qortoo.dt    — datatype key
qortoo.duid  — datatype unique ID
```

### Running locally with Jaeger

```shell
make enable-jaeger          # starts Jaeger via docker-compose
cargo test --all-features   # tests emit spans to Jaeger
# open http://localhost:16686
```

---

## Metrics

### Design: facade pattern

```
Qortoo-rs (library)
  └─ metrics::counter!("qortoo_sync_total", ...)   ← facade call
       └─ [global recorder, set by application]
            ├─ metrics-exporter-prometheus  (user's choice)
            ├─ metrics-exporter-statsd
            └─ no recorder → AtomicPtr no-op, zero cost
```

The `metrics` crate (`^0.24`) is always a dependency. Heavy exporters live in the application, not the library — no version conflicts, no forced backend.

### Metric catalogue

All metric names follow the Prometheus convention (`snake_case`, unit suffix).

#### `qortoo_sync_total` — counter

Counts every push/pull sync cycle.

| Label | Values | Description |
|-------|--------|-------------|
| `collection` | string | Collection name |
| `key` | string | Datatype key |
| `type` | `Counter` / … | CRDT type |
| `result` | `success` / `failure` | Outcome of the sync |

```
qortoo_sync_total{collection="orders",key="visits",type="Counter",result="success"} 42
qortoo_sync_total{collection="orders",key="visits",type="Counter",result="failure"} 1
```

#### `qortoo_sync_duration_seconds` — histogram

End-to-end latency of a single `push_pull()` call, in seconds. Includes both the connectivity round-trip and local state application.

| Label | Values |
|-------|--------|
| `collection` | string |
| `key` | string |
| `type` | CRDT type |

#### `qortoo_backoff_total` — counter

Incremented on each recoverable connectivity failure that results in a `BackOff` action, including retries that fail while already in backoff. Useful for alerting on degraded connectivity.

| Label | Values |
|-------|--------|
| `collection` | string |
| `key` | string |
| `type` | CRDT type |

### Instrumentation points

```
WiredDatatype::push_pull()            ← records sync_total + sync_duration_seconds
  └── do_push_pull()
        └── connectivity.push_and_pull()

EventLoop::run()  [PushTransaction error path]
  └── if event_loop_action == BackOff  ← records backoff_total
```

### Connecting to Prometheus

In the application, install a recorder once at startup:

```toml
# application Cargo.toml
metrics-exporter-prometheus = "0.16"
```

```rust
use metrics_exporter_prometheus::PrometheusBuilder;

fn main() {
    // registers the global recorder; all metrics::counter!() calls route here
    let handle = PrometheusBuilder::new()
        .install_recorder()
        .unwrap();

    // expose /metrics endpoint (example with axum)
    let app = axum::Router::new()
        .route("/metrics", axum::routing::get(move || {
            let body = handle.render();
            async move { body }
        }));
    // ...
}
```

The `/metrics` endpoint is then scraped by Prometheus:

```yaml
# prometheus.yml
scrape_configs:
  - job_name: qortoo-app
    static_configs:
      - targets: ['localhost:3000']
    scrape_interval: 15s
```

### Testing metrics

Unit and integration tests use `metrics-util`'s `DebuggingRecorder` — an in-memory recorder that captures metric values without any network or file I/O.

```toml
# dev-dependencies only
metrics-util = { version = "^0.20", features = ["debugging"] }
```

```rust
use metrics_util::debugging::DebuggingRecorder;

let recorder = DebuggingRecorder::new();
let snapshotter = recorder.snapshotter();
recorder.install().unwrap();  // sets the global recorder for this process

// ... trigger sync ...

let snap = snapshotter.snapshot().into_vec();
// inspect snap for expected counter/histogram values
```

**Important — metrics-util 0.20 semantics:**
`snapshot()` resets (consumes) **all** registered metrics to zero — counters and histograms alike. This is a breaking change from 0.18 which only drained histograms.

Consequences for test design:

| Rule | Reason |
|------|--------|
| Call `snapshot()` exactly once per assertion group | Each call resets everything; two calls drain each other's values |
| Mark metrics tests `#[serial_test::serial]` | Parallel tests share the global recorder; one test's snapshot resets another's counters |
| Use `extract_counter!` on an already-drained vec | Avoids a second snapshot that would lose histogram samples |

See `src/observability/metrics.rs` (`tests_metrics` module) for the full test pattern.

---

## Key Types Quick Reference

| Type | Location | Purpose |
|------|----------|---------|
| `add_span_event!` | `src/observability/macros.rs` | Add an OpenTelemetry span event at the current tracing scope |
| `metrics::emit_sync` | `src/observability/metrics.rs` | Emit `sync_total` + `sync_duration_seconds` for one push/pull |
| `metrics::emit_backoff` | `src/observability/metrics.rs` | Emit `backoff_total` on BackOff entry |
| `SYNC_TOTAL`, `BACKOFF_TOTAL`, … | `src/observability/metrics.rs` | Metric name constants (module-private; single source of truth within the module) |

