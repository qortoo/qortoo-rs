//! Rust half of the Rust/Go benchmark pair.
//!
//! Every scenario here has a counterpart of the same name in
//! `go/qortoo/benchmark_test.go`, so subtracting the two ns/op figures yields the
//! cost of the cgo binding for that operation. Keep the two files in sync: a
//! change to a workload or a budget on one side is meaningless unless it is
//! mirrored on the other. See `docs/performance.md` for the recorded baselines
//! and the measurement protocol.
//!
//! Common conditions (mirrored in Go): the connectivity backend is
//! `LocalConnectivity` in manual mode (`set_realtime(false)`) so no background
//! sync interferes, measurements are single-threaded, and no handler is
//! registered — the handler trampoline cost is represented by the transaction
//! scenario instead.
//!
//! # Why a fixed budget instead of criterion
//!
//! Cost per operation is not constant in this SDK. A datatype accumulates its
//! pending push buffer and its server-side history; a client accumulates every
//! datatype ever registered in it and never releases the tokio worker threads of
//! a dropped client. Every one of those makes later operations slower, so what a
//! run reports depends on how many iterations the harness chose to run — and
//! criterion (time-driven) picked roughly an order of magnitude more iterations
//! than `go test -bench` did, which made the two halves incomparable in exactly
//! the way this benchmark exists to avoid.
//!
//! So both sides run a pinned budget instead: `BUDGET` operations per round,
//! `ROUNDS` rounds, datatype replaced every `RESET_EVERY_*` operations outside
//! the measured region. Run the Go side with the matching `-benchtime <budget>x`
//! (`make bench-go` does).

use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use qortoo::{Client, Counter, Datatype, LocalConnectivity};

const COLLECTION: &str = "bench-collection";

/// Operations recorded per transaction in the transaction scenario.
const TX_OPS: usize = 10;

/// Independent rounds per scenario; the spread across rounds is the noise floor.
const ROUNDS: usize = 10;

/// Single writes performed on one datatype before it is replaced.
const RESET_EVERY_OPS: u64 = 50_000;

/// Transactions performed on one datatype before it is replaced. Lower than
/// `RESET_EVERY_OPS` by `TX_OPS` so the same number of operations is buffered.
const RESET_EVERY_TXS: u64 = RESET_EVERY_OPS / TX_OPS as u64;

/// Push-pulls performed on one datatype before it is replaced. The server keeps
/// every pushed transaction in its history, which is what this bounds.
const RESET_EVERY_SYNCS: u64 = 5_000;

/// Per-round budgets. Each must match the `-benchtime <n>x` used for the
/// corresponding Go benchmark.
const BUDGET_INCREASE_BY: u64 = 200_000;
const BUDGET_GET_VALUE: u64 = 2_000_000;
const BUDGET_TRANSACTION: u64 = 20_000;
const BUDGET_SYNC_LOCAL: u64 = 20_000;
const BUDGET_BUILD_COUNTER: u64 = 100;

/// Builds a client whose connectivity never syncs on its own.
fn new_client(alias: &str) -> Client {
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    Client::builder(COLLECTION, alias)
        .with_connectivity(connectivity)
        .build()
        .expect("client must build")
}

fn new_counter(client: &Client, key: &str) -> Counter {
    client
        .create_datatype(key)
        .build_counter()
        .expect("counter must build")
}

/// Runs `budget` operations on `client`, replacing the datatype every
/// `reset_every` operations and timing only the operations themselves.
fn timed_with_reset(
    client: &Client,
    round: usize,
    budget: u64,
    reset_every: u64,
    mut op: impl FnMut(&Counter),
) -> Duration {
    let mut elapsed = Duration::ZERO;
    let mut done = 0;
    while done < budget {
        let chunk = (budget - done).min(reset_every);
        // Keys must be unique: a key can only be registered once per client.
        let counter = new_counter(client, &format!("r{round}-{done}"));
        let start = Instant::now();
        for _ in 0..chunk {
            op(&counter);
        }
        elapsed += start.elapsed();
        done += chunk;
    }
    elapsed
}

/// Runs `ROUNDS` rounds of `budget` operations, writing one Go benchmark format
/// line per round to stdout and a human summary to stderr.
///
/// `name` must match the Go benchmark of the same scenario exactly (minus its
/// `Benchmark` prefix): `benchstat -col /impl` pairs the two halves by name, and
/// a mismatch silently turns the comparison into two unrelated rows.
fn scenario(name: &str, budget: u64, mut round: impl FnMut(usize) -> Duration) {
    // One discarded round: the machine is still settling when the first scenario
    // starts (the build just finished, CPU clocks are ramping), which otherwise
    // shows up as a two- to threefold outlier in the first samples. The Go half
    // runs later in `make bench` and does not need one.
    round(usize::MAX);

    let mut per_op = Vec::with_capacity(ROUNDS);
    for r in 0..ROUNDS {
        let ns = round(r).as_nanos() as f64 / budget as f64;
        println!("Benchmark{name}/impl=rust\t{budget}\t{ns:.2} ns/op");
        per_op.push(ns);
    }
    per_op.sort_by(f64::total_cmp);
    eprintln!(
        "{name:<16} {budget:>9} x {ROUNDS:<3}  {:>10.2} {:>10.2} {:>10.2}",
        per_op[0],
        per_op[ROUNDS / 2],
        per_op[ROUNDS - 1],
    );
}

/// Go's `GOOS` spelling of the current platform, so a merged result file has one
/// consistent configuration for both halves.
fn go_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    }
}

/// Go's `GOARCH` spelling of the current architecture.
fn go_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "amd64",
        other => other,
    }
}

/// Single write: the cheapest mutating operation, dominated by CRDT bookkeeping.
fn bench_increase_by() {
    let client = new_client("bench-increase-by");
    scenario("IncreaseBy", BUDGET_INCREASE_BY, |round| {
        timed_with_reset(
            &client,
            round,
            BUDGET_INCREASE_BY,
            RESET_EVERY_OPS,
            |counter| {
                black_box(
                    counter
                        .increase_by(black_box(1))
                        .expect("increase must succeed"),
                );
            },
        )
    });
}

/// Single read: the cheapest call of the whole API, where the relative weight of
/// the binding is largest.
fn bench_get_value() {
    let client = new_client("bench-get-value");
    let counter = new_counter(&client, "get-value");
    counter.increase().expect("increase must succeed");
    scenario("GetValue", BUDGET_GET_VALUE, |_round| {
        let start = Instant::now();
        for _ in 0..BUDGET_GET_VALUE {
            black_box(black_box(&counter).get_value());
        }
        start.elapsed()
    });
}

/// Transaction committing `TX_OPS` operations. In Go this additionally crosses
/// the callback trampoline (Go → Rust → Go), which is the point of the pair.
fn bench_transaction() {
    let client = new_client("bench-transaction");
    scenario("Transaction", BUDGET_TRANSACTION, |round| {
        timed_with_reset(
            &client,
            round,
            BUDGET_TRANSACTION,
            RESET_EVERY_TXS,
            |counter| {
                counter
                    .transaction("bench", |tx| {
                        for _ in 0..TX_OPS {
                            tx.increase_by(1)?;
                        }
                        Ok(())
                    })
                    .expect("transaction must commit");
            },
        )
    });
}

/// One write plus a manual push-pull against the in-memory server: the protocol
/// path, where the binding should be a rounding error.
fn bench_sync_local() {
    let client = new_client("bench-sync-local");
    scenario("SyncLocal", BUDGET_SYNC_LOCAL, |round| {
        timed_with_reset(
            &client,
            round,
            BUDGET_SYNC_LOCAL,
            RESET_EVERY_SYNCS,
            |counter| {
                counter
                    .increase_by(black_box(1))
                    .expect("increase must succeed");
                counter.sync().expect("sync must succeed");
            },
        )
    });
}

/// Building and dropping a counter: the one-time cost per datatype, which the Go
/// counterpart pays together with its `runtime.AddCleanup` registration. Each
/// round gets a fresh client because the cost grows with the number of datatypes
/// already registered — and rounds still drift upward, since a dropped client
/// keeps its worker threads.
fn bench_build_counter() {
    scenario("BuildCounter", BUDGET_BUILD_COUNTER, |round| {
        let client = new_client(&format!("bench-build-counter-{round}"));
        let keys: Vec<String> = (0..BUDGET_BUILD_COUNTER)
            .map(|i| format!("build-{i}"))
            .collect();
        let start = Instant::now();
        for key in &keys {
            drop(black_box(new_counter(&client, key)));
        }
        start.elapsed()
    });
}

fn main() {
    // Go benchmark format: configuration lines first, then one line per sample.
    // The data goes to stdout so `make bench-rust > file` yields a file benchstat
    // can read; the human-readable summary goes to stderr.
    println!("goos: {}", go_os());
    println!("goarch: {}", go_arch());
    println!("pkg: qortoo");
    eprintln!("qortoo Rust benchmarks (fixed budget; pair with go/qortoo/benchmark_test.go)");
    eprintln!(
        "{:<16} {:>13}  {:>10} {:>10} {:>10}",
        "scenario", "budget x rounds", "min ns/op", "med ns/op", "max ns/op"
    );
    bench_increase_by();
    bench_get_value();
    bench_transaction();
    bench_sync_local();
    bench_build_counter();
}
