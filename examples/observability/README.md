# Observability Examples

These examples show how an application can export Qortoo telemetry to the local
Grafana observability stack.

| Example | Backend | Cargo feature | Run command |
|---------|---------|---------------|-------------|
| Trace | Tempo | `observability-trace` | `cargo run --features observability-trace --example trace` |
| Log | Loki | None; `observability-log` is optional | `RUST_LOG=info cargo run --example log` |
| Metrics | Prometheus | `observability-metrics` | `cargo run --features observability-metrics --example metrics` |
| Profile | Pyroscope | None | `cargo run --example profile` |

Run all commands in this document from the `qortoo-rs` crate root.

## Prerequisites

- Rust 1.87.0 or newer
- Docker with the Compose plugin
- `make`

Start Grafana, Prometheus, Tempo, Loki, and Pyroscope:

```shell
make obs-up
```

Open [Grafana](http://localhost:3000) and sign in with:

```text
username: admin
password: qortooAdmin
```

To inspect the containers:

```shell
docker compose -f qortoo-rs-docker/docker-compose.yml ps
make obs-logs
```

## Trace

The trace example installs Qortoo's OpenTelemetry pipeline, runs a two-client
counter synchronization, writes compact text logs through `QortooLogLayer`, exports its
spans to Tempo over OTLP gRPC, and exits. ANSI level colors are enabled only when stdout
is connected to a terminal.

```shell
cargo run --features observability-trace --example trace
```

The final counter values should both be `5`. In Grafana, select
**Explore → Tempo → Search** and filter by the service name
`qortoo-example-trace`.

The default OTLP endpoint is `http://localhost:4317`. Override it with the
trace-specific variable or the generic OpenTelemetry variable:

```shell
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://my-collector:4317 \
  cargo run --features observability-trace --example trace
```

```shell
OTEL_EXPORTER_OTLP_ENDPOINT=http://my-collector:4317 \
  cargo run --features observability-trace --example trace
```

`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT` takes precedence when both variables are
set.

## Log

The log example writes tracing events to stdout and ships them to Loki at
`http://localhost:3100`. Run it with the standard `tracing_subscriber` formatter:

```shell
RUST_LOG=info cargo run --example log
```

To use Qortoo's compact `QortooLogLayer` formatter instead:

```shell
RUST_LOG=info cargo run --features observability-log --example log
```

The example exits after printing a counter value of `3`. In Grafana, select
**Explore → Loki** and run this LogQL query:

```logql
{app="qortoo", source="example"}
```

Change `RUST_LOG` to control verbosity, for example:

```shell
RUST_LOG=qortoo=trace,log=info cargo run --features observability-log --example log
```

## Metrics

The metrics example installs a Prometheus recorder, exposes
`http://localhost:9000/metrics`, and updates a counter every five seconds.

```shell
cargo run --features observability-metrics --example metrics
```

Leave the process running. From another terminal, confirm that the scrape
endpoint is serving Qortoo metrics:

```shell
curl --fail http://localhost:9000/metrics
```

Prometheus scrapes the endpoint every 15 seconds. In Grafana, select
**Explore → Prometheus** and query:

```promql
qortoo_sync_total
```

Press <kbd>Ctrl</kbd>+<kbd>C</kbd> to stop the example.

The bundled Prometheus configuration uses
`host.docker.internal:9000`, which works with Docker Desktop on macOS and
Windows. On Linux Docker Engine, change the target in
`qortoo-rs-docker/prometheus/prometheus.yml` to the Docker bridge gateway,
commonly `172.17.0.1:9000`, and restart Prometheus.

## Profile

The profile example runs a CPU-intensive two-client counter workload, exports
pprof samples to Pyroscope, and exits after 30 seconds by default.

```shell
cargo run --example profile
```

In Grafana, select **Explore → Pyroscope** and choose the application
`qortoo-example-profile`.

The endpoint, application name, and workload duration are configurable:

```shell
PYROSCOPE_URL=http://localhost:4040 \
PYROSCOPE_APPLICATION_NAME=qortoo-example-profile \
QORTOO_PROFILE_SECONDS=60 \
  cargo run --example profile
```

## Stop the Stack

Stop the containers while keeping their persisted data:

```shell
make obs-down
```

Stop the containers and delete the Prometheus, Grafana, Tempo, Loki, and
Pyroscope volumes:

```shell
make obs-down-v
```

## Troubleshooting

- If Cargo reports that the `trace` or `metrics` target requires a feature, add
  `--features observability-trace` or `--features observability-metrics` to the
  matching command. The `observability` umbrella also satisfies both.
- If an exporter reports `connection refused`, check that `make obs-up`
  completed and that the relevant port is available: Grafana `3000`,
  Prometheus `9090`, Loki `3100`, Tempo `4317`, Pyroscope `4040`, and the
  metrics example `9000`.
- If a metrics query is empty, keep the example running for at least one
  15-second Prometheus scrape interval and verify
  `http://localhost:9000/metrics` first.
- If Loki has no matching logs, run the example with `RUST_LOG=info` and verify
  the `{app="qortoo", source="example"}` query.

For the telemetry API, metric catalogue, and instrumentation details, see
[`../../docs/observability.md`](../../docs/observability.md).
