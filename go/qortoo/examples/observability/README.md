# Go Observability Examples

These examples are the Go-binding counterparts of the Rust programs in
[`../../../../examples/observability`](../../../../examples/observability).
They export Qortoo telemetry to the local Grafana observability stack.

| Example | Backend | Run command |
|---------|---------|-------------|
| Trace | Tempo | `go run ./examples/observability/trace` |
| Log | stdout | `go run ./examples/observability/log` |
| Metrics | Prometheus | `go run ./examples/observability/metrics` |
| Profile | Pyroscope | `go run ./examples/observability/profile` |

Run all commands in this document from `go/qortoo`.

## Prerequisites

Build the native library used by cgo and start the local observability stack:

```shell
cd ../..
cargo build -p qortoo-ffi
make obs-up
cd go/qortoo
```

Open [Grafana](http://localhost:3000) and sign in with `admin` / `qortooAdmin`.

## Trace

The trace example installs both the Go and Qortoo OTLP exporters. A Go root span is
passed to `SyncContext`, so the Rust FFI and event-loop spans join the same trace.

```shell
go run ./examples/observability/trace
```

The final counter values should both be `5`. In Grafana, select
**Explore -> Tempo -> Search** and filter by the service name
`qortoo-go-example-trace`.

The default OTLP endpoint is `http://localhost:4317`. Both exporters honor
`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, then `OTEL_EXPORTER_OTLP_ENDPOINT`:

```shell
OTEL_EXPORTER_OTLP_TRACES_ENDPOINT=http://my-collector:4317 \
  go run ./examples/observability/trace
```

## Log

The Go binding can ask the Rust core to emit structured JSON or compact text logs.
This example selects text logs and runs a counter operation:

```shell
go run ./examples/observability/log
```

Set `QORTOO_LOG_FORMAT=json` for newline-delimited JSON, or set `RUST_LOG` to
change verbosity:

```shell
QORTOO_LOG_FORMAT=json RUST_LOG=qortoo=trace \
  go run ./examples/observability/log
```

Unlike the Rust example, the Go binding does not expose the Rust subscriber as a
composable layer. It therefore writes Qortoo logs to stdout; a deployment can ship
that stream to Loki with its normal log collector.

## Metrics

The metrics example exposes `http://localhost:9000/metrics` and updates a counter
every five seconds:

```shell
go run ./examples/observability/metrics
```

Confirm the endpoint from another terminal:

```shell
curl --fail http://localhost:9000/metrics
```

In Grafana, select **Explore -> Prometheus** and query `qortoo_sync_total`.
Press <kbd>Ctrl</kbd>+<kbd>C</kbd> to flush and stop the exporter. Override the
listen address with `QORTOO_METRICS_LISTEN_ADDR`.

## Profile

The profile example runs a CPU-intensive two-client counter workload and sends Go
pprof samples to Pyroscope for 30 seconds:

```shell
go run ./examples/observability/profile
```

In Grafana, select **Explore -> Pyroscope** and choose
`qortoo-go-example-profile`. The endpoint, application name, and duration are
configurable:

```shell
PYROSCOPE_URL=http://localhost:4040 \
PYROSCOPE_APPLICATION_NAME=qortoo-go-example-profile \
QORTOO_PROFILE_SECONDS=60 \
  go run ./examples/observability/profile
```

## Stop the Stack

From the repository root:

```shell
make obs-down
```
