# Rust Core Performance

The Rust core ships a benchmark harness that measures a fixed number of operations per round
rather than running for a fixed time. Its purpose is comparison — between two revisions of the
core, and between the core and a language binding measuring the same workloads — so the shape
of a run is a contract rather than a convenience. This document defines that contract, what a
measurement does and does not tell you, and how to read a result. The cross-repository tooling
that stores and compares results lives with the Go binding.

## Model

| Term | Meaning |
|------|---------|
| Scenario | One named workload, measured on its own |
| Budget | The number of operations one round of a scenario performs; fixed per scenario |
| Round | One measured execution of a scenario's budget |
| Reset interval | How often the datatype under test is replaced, outside the measured interval |
| Contract | The scenario names, budgets, workloads, and reset intervals that a comparable run must match |

A run executes each scenario for a fixed number of rounds and reports the time per operation
for each. Because the budget is fixed rather than the duration, two implementations of the same
scenario perform exactly the same amount of work, and their numbers describe the same
experiment.

## Rules and Guarantees

**The budget is part of the contract.** Scenario names, per-round budgets, the workload each
performs, and the reset intervals together define what a result means. Anything measuring
itself against this harness must match all of them; a run with a different budget is a
different experiment and its numbers are not comparable.

**State is bounded, outside the measurement.** A datatype accumulates unsent work and a backend
accumulates history, and per-operation cost grows with both. Mutable scenarios therefore replace
the datatype periodically, and that replacement happens outside the measured interval so it does
not enter the numbers.

**The measured conditions are fixed.** The local backend runs in manual mode, the measured loops
are single-threaded, and no handler is registered. A measurement describes the core's own cost,
not the cost of a scheduling policy or of application callbacks.

**The first round is discarded.** Each scenario emits ten rounds and the first is treated as
warm-up rather than as a measurement.

**Output is in a portable format.** Results are emitted in the Go benchmark format, tagged with
the implementation, so existing tooling can consume them and a binding can place its own numbers
beside the core's without a custom parser.

**Benchmarks do not gate shared CI.** Hosted runners are too noisy at this resolution. A change
that could affect performance is measured deliberately, on a known host, and the result is
reported with its metadata.

**Limits on the guarantees.** A number is meaningful only against another number from the same
host, the same budget, and a comparably quiet system. The harness measures the Rust core through
its public API; it does not isolate individual layers, and it does not measure anything about a
real network, since the backend it uses is in-process.

## Scenarios

| Scenario | Budget per round | Workload |
|----------|-----------------:|----------|
| `IncreaseBy` | 200,000 | One counter write |
| `GetValue` | 2,000,000 | One counter read |
| `Transaction` | 20,000 | One transaction containing 10 writes |
| `SyncLocal` | 20,000 | One write and one manual exchange |
| `BuildCounter` | 100 | Build and drop one counter |

The budgets differ by three orders of magnitude because the operations do: a read that touches
only local state and a build that constructs a datatype and starts its event loop are not
comparable units of work, and giving each a budget that runs for a sensible interval keeps every
scenario's rounds in a usable range.

## Behavior

**Running the harness.** `make bench` runs it. The harness supplies its own entry point rather
than using the default benchmark runner, which is what lets it control the round structure and
the output format.

**Reading a result.** Look at the spread across the rounds before drawing a conclusion from any
single number. A change that moves the mean by less than the spread is not yet evidence of
anything.

**Comparing against a previous result.** Compare only measurements from the same host, with the
same budgets, on a system doing nothing else. `SyncLocal` and `BuildCounter` are the most
sensitive to whatever else the machine is doing, because both do work whose cost is not entirely
the core's.

**Comparing a language binding against the core.** The binding measures the same scenario names
with the same budgets and workloads, and the two sets of numbers go side by side. The tooling
that runs both, records which revisions were used, and stores results belongs to the Go binding:
see [`qortoo-go/docs/performance.md`](https://github.com/qortoo/qortoo-go/blob/main/docs/performance.md).
It requires explicit paths to both checkouts and records both revisions, the native SDK and ABI
version, and host information alongside every result.

**Looking for a regression.** Allocation counts move less between runs than wall-clock time
does, so they are usually the clearer signal that something changed.

## Rationale

**The budget is fixed rather than the duration.** A time-driven harness chooses its own
iteration count, so a faster implementation performs more operations per round than a slower
one. Here that would place them in different cost regimes — more operations mean more
accumulated state, and per-operation cost grows with it — so the faster implementation would be
measured under conditions the slower one never reached. Fixing the budget makes the two runs the
same experiment.

**The datatype is replaced periodically.** Without it, a long run measures the cost of an
increasingly large push buffer and history rather than the cost of the operation. Replacing the
datatype keeps the measured region in a steady state; doing it outside the measured interval
keeps the replacement out of the numbers.

**The measured configuration is deliberately minimal.** Realtime pushing, background handlers,
and concurrent callers all add cost that belongs to a scheduling policy or to application code.
Excluding them makes a change in the numbers attributable to the core.

**The output format is borrowed rather than invented.** Reusing an established benchmark format
means the existing comparison tools work, and a binding in another language can emit the same
format instead of agreeing on a new one.

**Benchmarks are not a CI gate.** At this resolution a shared runner's noise exceeds the effects
worth detecting, so a gate would either fail on noise or be set so loose it catches nothing. A
deliberate measurement on a known host is the only kind that carries information.

## Code Map

| Concern | Location |
|---------|----------|
| The harness, its own entry point, and the round structure | `benches/qortoo_bench.rs` |
| Budgets, reset intervals, transaction size, and round count | `benches/qortoo_bench.rs` (the `BUDGET_*`, `RESET_EVERY_*`, `TX_OPS`, `ROUNDS` constants) |
| The command that runs it | `Makefile` (`bench`) |
| The backend the sync scenario exchanges through | `src/connectivity/local_connectivity.rs` |
| Cross-repository comparison, result metadata, and storage | [`qortoo-go/docs/performance.md`](https://github.com/qortoo/qortoo-go/blob/main/docs/performance.md) |

## Related Concepts

- [`docs/architecture.md`](architecture.md) — the layers an operation passes through
- [`docs/connectivity.md`](connectivity.md) — the manual local backend the sync scenario uses
- [`docs/transaction-and-rollback.md`](transaction-and-rollback.md) — the transaction workload and the push buffer that bounds it
- [`docs/client-and-datatype-builder.md`](client-and-datatype-builder.md) — what building a datatype does, which one scenario measures
- [`docs/go-binding.md`](go-binding.md) — the boundary a language binding measures across
