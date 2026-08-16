# Rust Core Performance

The benchmark target in `benches/qortoo_bench.rs` measures five fixed-budget scenarios
against the Rust core. Run it with:

```shell
make bench
# equivalent to: cargo bench --bench qortoo_bench
```

The harness has its own `main` (`harness = false`) and emits Go benchmark format tagged
with `impl=rust`. This makes the output consumable by benchstat and lets a language
binding pair its own measurements with the Rust baseline without a custom parser.

## Scenarios

| Scenario | Budget per round | Workload |
| --- | ---: | --- |
| `IncreaseBy` | 200,000 | One counter write |
| `GetValue` | 2,000,000 | One counter read |
| `Transaction` | 20,000 | One transaction containing 10 writes |
| `SyncLocal` | 20,000 | One write and one manual push/pull |
| `BuildCounter` | 100 | Build and drop one counter |

Each scenario emits ten measured rounds. The first Rust round is discarded as warm-up.
Mutable scenarios periodically replace the datatype outside the measured interval so
pending operations and server-side history remain bounded. `LocalConnectivity` runs in
manual mode, the measured loops are single-threaded, and no handler is registered.

## Why the Budget Is Fixed

Per-operation cost grows with accumulated state. A datatype retains its pending push
buffer and server history; a client also retains registered datatypes. A time-driven
harness would choose different iteration counts for implementations with different
throughput and put them in different cost regimes.

The `BUDGET_*`, `RESET_EVERY_*`, and `TX_OPS` constants therefore form part of the
benchmark contract. A language binding that compares itself with this harness must use
the same scenario name, workload, reset interval, and operation budget.

The Go binding owns the cross-repository contract check, release-link benchmark runner,
result metadata, saved-result comparison, and Bencher upload workflow. See
[`qortoo-go/docs/performance.md`](https://github.com/qortoo/qortoo-go/blob/main/docs/performance.md).
That tool requires explicit paths to both checkouts and records both commit SHAs, the
native SDK/ABI version, and host information next to every result.

## Interpreting Results

- Compare measurements only from the same host and a quiet system.
- Read the spread across rounds before interpreting a wall-clock change.
- `SyncLocal` and `BuildCounter` are especially sensitive to host and runtime noise.
- Treat different operation budgets as different experiments.
- Allocation counts are often a more stable regression signal than nanoseconds per
  operation.

Wall-clock benchmarks do not gate shared CI because hosted runners are too noisy at this
resolution. Run the relevant scenario manually for changes that can affect performance
and attach the result metadata with the comparison.

## Related Concepts

- [Architecture](architecture.md) — the layers traversed by each operation
- [Connectivity](connectivity.md) — the manual local backend used by `SyncLocal`
- [Transaction and Rollback](transaction-and-rollback.md) — the transaction workload
- [C ABI and Native SDK](go-binding.md) — the boundary measured by language bindings
