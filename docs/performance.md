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
