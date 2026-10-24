# Performance

[Qortoo Bench](https://github.com/qortoo/qortoo-bench) owns the Rust and Go
benchmark workloads, measurement contracts, execution, and official result data.
The SDK owns product code, functional tests, and native SDK/ABI validation.

## Run and inspect measurements

Use the central repository's [local run instructions](https://github.com/qortoo/qortoo-bench#run-full-measurements)
or its [manual GitHub workflow](https://github.com/qortoo/qortoo-bench/blob/main/docs/central-workflow.md).
The central source lock selects exact Rust and Go revisions; a local SDK worktree
is not automatically the measured input. Each result identifies both SDK commits,
the harness, native SDK checksums, measurement environment, and raw evidence.

The contracts cover [Counter](https://github.com/qortoo/qortoo-bench/blob/main/docs/counter-measurement.md)
and [Variable](https://github.com/qortoo/qortoo-bench/blob/main/docs/variable-measurement.md).
Changes to workload, budgets, reset intervals, or timing boundaries belong there.

## Interpret results

Read the central [result guide](https://github.com/qortoo/qortoo-bench/blob/main/docs/results.md)
and [metric/series contract](https://github.com/qortoo/qortoo-bench/blob/main/docs/result-publishing.md).
Rust/Go latency differences describe their complete public API paths, including
serialization and runtime work; they do not isolate the cost of a binding call.
Compare trends only within matching contracts, toolchains, and environments.
Smoke runs check execution and state, not performance.

For client/datatype scaling, resident memory, idle threads, and shutdown observations,
see [Runtime Resources](runtime-resources.md). Its process-isolated resource probe
is separate from latency benchmarks and remains an SDK development tool.

## Automatic main measurements

Pushes to `main` request central benchmarks for code, dependency, build, or
validation changes. Documentation-only and repository metadata changes are skipped
according to the [workflow filters](../.github/workflows/benchmark.yml).

Each request identifies the exact push SHA, and the central ledger preserves its
dependency selection and outcomes. For manual measurements and recovery, see the
[central workflow guide](https://github.com/qortoo/qortoo-bench/blob/main/docs/central-workflow.md).

PRs targeting `main` request a separate [candidate base/head comparison](https://github.com/qortoo/qortoo-bench/blob/main/docs/pr-candidates.md)
after the PR event workflow succeeds. Fork PRs follow the source repository's
Actions approval policy. The candidate result never updates official history or
website performance data and does not block merging based on a numeric threshold.
Manual central runs remain available. Central latency measurements do not measure
heap allocations.

The PR notifier locates open PRs by the source run's head owner and branch, then
requires an exact repository, branch, and commit match. Recovery of an old head
skips the request instead of substituting the PR's latest commit.
