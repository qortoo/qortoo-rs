# Runtime Resources and Callback Progress

Each client owns a Tokio runtime with `available_parallelism()` async workers. Each live
datatype starts one long-running `spawn_blocking` event loop. Its channel receive blocks
an OS thread even when there is no synchronization work. Consequently, idle SDK thread
cost is approximately `clients × workers + live datatypes`, in addition to application
threads. Tokio can retain idle blocking threads after a loop exits, so handle count alone
does not give the instantaneous thread count.

## Callback Contract

Handlers are synchronous closures scheduled on the owning client's async workers. They
are not run on the event-loop thread, and the dispatcher does not await their completion.
Mutable state is released before scheduling; the effects gate protects dispatch order,
not callback execution or completion order. See [Handler System](handler-system.md).

| Callback action | Contract |
|-----------------|----------|
| Read the supplied datatype | Reads current state, which may include later transitions or uncommitted local writes |
| Write or start a transaction | Uses the ordinary transaction gate; may wait for an unrelated transaction |
| Call `sync()` after a transaction returns | Waits for the separate blocking event loop to apply the response; does not wait for other handlers |
| Call `sync()` inside a transaction callback | Rejected on the transaction's owner thread; see [Transaction and Rollback](transaction-and-rollback.md) |
| Unsubscribe and sync | Can complete from a handler; later lifecycle notifications are separate tasks |
| Block for a long time | Occupies one async worker; later priorities in this notification wait, while unrelated API work and the blocking event loop can progress |
| Drop external client/datatype handles | An already dispatched notification keeps its datatype and runtime owner alive until its callbacks finish |

Blocking every async worker prevents other handler tasks on that client from starting.
A callback must not wait for another notification whose execution requires the worker it
occupies, or for work blocked behind its own transaction. No bounded handler latency is
promised for arbitrary user callbacks. Offload long application work and avoid circular
waits. Different clients have independent runtimes, but still share process resources.

## Shutdown and Backend Ownership

Dropping a `Client` removes its manager ownership; outstanding datatype handles and
notifications can still retain the client common state. Dropping the last transactional
handle requests event-loop stop. That request waits for acknowledgement for at most
100 ms; acknowledgement precedes loop exit and is not an OS-thread join. An exchange
already blocked inside a backend can delay stop processing. When the last client common
reference disappears, runtime shutdown is requested in the background. Started blocking
tasks cannot be forcibly cancelled, and neither runtime shutdown nor handle drop promises
completion of every queued notification.

Backend registration does not own a client's runtime. `LocalConnectivity` holds weak
wired references; the event loop owns its wired instance until it exits. The per-resource
backend owns its accepted CRDT and generates subscription snapshots without upgrading client
references. Dropping external handles can release client runtime ownership without an explicit
unsubscribe. Already dispatched callbacks and in-flight exchanges can delay that release under
the contracts above.

While the backend itself is retained, dropping client handles preserves server CRDT state, history
and subscription records for later subscribers. Use `unsubscribe()` and `sync()` to
remove a subscription explicitly; the resource is removed when all subscriptions and
registrations have been removed. Dropping the backend releases its stored records.
`NullConnectivity` does not store wired instances.

## Reproducible Resource Probe

Build and measure sequentially with the Makefile target. Run it without other builds
or tests in parallel:

```sh
make measure-resources
# Override the result path or measurement budget:
make measure-resources RESOURCE_OUTPUT=/tmp/qortoo-resources.json RESOURCE_ROUNDS=1 RESOURCE_ITERATIONS=100
```

The defaults are `target/resource-measurements.json`, three rounds, and 500 samples
per API path. The output file is replaced on each run. This target builds with default
features before invoking the Python driver; it is separate from the central latency benchmarks and CI.
The driver also remains directly callable for custom builds and sampling options.

The Python driver supports macOS and Linux with `ps`, uses a fresh process per case,
alternates case order over three rounds, and captures three RSS/thread samples at each
lifecycle checkpoint. It separately scales clients (1–16) and datatypes per client
(0–128), with a combined 4-client/32-datatype case. The main backend is manual
`LocalConnectivity`; additional local-drop and default `NullConnectivity` cases expose
different ownership and synchronization policies. No telemetry subscriber/exporter is
installed. Build with `--all-features` and record that choice with `--features all` to
measure feature compilation separately from an actively running telemetry pipeline.
The local drop-only control takes an additional sample after an 11-second wait to
distinguish retained runtime workers from Tokio's idle blocking-thread cache.

Checkpoints distinguish baseline, client construction, initialized idle datatypes,
completed workload, unsubscribe cleanup, dropped client owners, and released external
handles/backend. Every checkpoint waits for the driver's sampling acknowledgement.
OS threads and RSS come from `ps`; RSS is resident memory, not reserved stack address
space or exact live allocations. Allocator retention can keep RSS above baseline after
successful thread reclamation. The JSON records host, compiler, revision, dirty state, binary hash,
build features, iteration budget, settling interval, and UTC timestamps. Rebuild the
binary after source changes; feature metadata is supplied by the caller.

API latencies are single-caller round-robin samples: read, standalone write, transaction
with one write, and write plus synchronous exchange. Each path is warmed up; buffered
writes are drained before measured exchanges. These distributions include public API
and local backend costs, and do not measure concurrent throughput, network delay, or
handler dispatch latency. Build and cleanup totals are separate from operation latency.
Resource samples after release establish observation times, not a guarantee of future
shutdown latency. Timeouts report a stalled probe instead of silently treating it as a
successful measurement. The probe is informational and is not a CI resource gate.

Public callback contracts are exercised in `tests/runtime_contract.rs`, with channels
controlling callback entry and release. A blocked callback can retain the last local
handle after explicit unsubscribe while external operations finish, and the notification
continues through later priorities before releasing its handler resources.

## Measured Operating Profile

A release build measured on 2026-10-11 used macOS 27.0.1, Mac16,8 (24 GiB RAM),
12-way available parallelism, Rust 1.97.1, default features (empty), and no installed
telemetry. The core revision was `311df8b`; the probe and contract documentation were
local additions. Each case ran in three fresh processes, with 500 samples per API path.
RSS growth below is the median idle increase over that process's baseline. Sync p95
is the range of per-round percentiles for one write plus a manual local exchange.

| Clients × datatypes per client | Idle threads | Idle RSS growth | Write + sync p95 |
|--------------------------------|-------------:|----------------:|-----------------:|
| 1 × 1 | 14 | 1.61 MiB | 25.5–31.2 µs |
| 1 × 128 | 141 | 4.77 MiB | 10.9–17.5 µs |
| 4 × 32 | 177 | 5.61 MiB | 19.1–21.5 µs |
| 16 × 1 | 209 | 6.30 MiB | 13.5–22.8 µs |

The exploratory budget is at most four clients and 128 total Counter datatypes,
192 idle process threads, 64 MiB sampled RSS growth, local write+sync p95 under 5 ms,
and a return to baseline threads within two seconds after releasing the final external
references following successful unsubscribe/sync. At most one long callback per client
is included in this profile; arbitrary blocking callbacks and concurrent load are outside
the measured contract. This is an evaluation envelope on this host, not a product SLA.
Intermediate client/datatype combinations follow the inspected thread-cost model but
were not all independently measured.

The representative cases within that envelope passed the budgets. The highest sampled
RSS increase was 6.33 MiB. Normal cleanup API time summed across unsubscribe/sync,
client drop, and final handle drop was at most 8.00 ms; this excludes driver sampling
pauses. Baseline threads were already restored at the first post-release observation,
0.311–0.325 seconds after that checkpoint. Sixteen clients exceeded the thread budget.
These figures cannot predict large stored values, network stalls, or contention.

An ownership validation run on the same host used default features, 12 cases across
three rounds, and 500 samples per API path. All release observations returned to the
one-thread baseline, including local drop-only and Null drop. Local drop-only reached
baseline at the first observation, 0.315–0.328 seconds after release, and remained there
at the additional observation about 11.5 seconds later in all three rounds. These are
observation times, not an exact thread-join deadline. Within the operating envelope,
idle threads were at most 177, sampled RSS growth at most 6.38 MiB, write+sync p95 at
most 28.7 microseconds, and normal cleanup API time at most 9.03 ms. Explicit unsubscribe
removes server subscription records; it is not required to release client runtime ownership.

The runtime structure is retained for this profile. Runtime sharing/injection would
reduce the per-client worker cost but needs a separate shutdown-ownership design.
Asynchronous event waiting would address the per-datatype thread cost but requires
adapting synchronous exchanges. Simply lowering the blocking-pool cap can queue live
event loops indefinitely, preventing sync and stop progress, so it is not a safe remedy
for the existing long-running-loop model. Reassess these choices when required client
or datatype counts exceed the measured envelope.

## Code Map

| Concern | Location |
|---------|----------|
| Runtime workers and background shutdown | `src/utils/runtime.rs` |
| Runtime ownership | `src/clients/common.rs` |
| Blocking event loop and stop acknowledgement | `src/datatypes/event_loop.rs` |
| Notification lifetime and synchronous handler execution | `src/datatypes/handler.rs` |
| Local backend weak registration and backend-owned snapshots | `src/connectivity/local_datatype_server.rs` |
| Isolated public API probe and OS sampling | `examples/resource_probe.rs`, `scripts/measure_resources.py` |

See [Performance](performance.md) for the separate fixed-budget Rust/Go comparison
contract, and [Connectivity](connectivity.md) for exchange and subscription behavior.
