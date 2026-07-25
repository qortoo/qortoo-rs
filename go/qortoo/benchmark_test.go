package qortoo

// Go half of the Rust/Go benchmark pair.
//
// Every benchmark here mirrors a scenario in `benches/qortoo_bench.rs`, so the
// difference between the two ns/op figures is the cost of the cgo binding for
// that operation. Keep the two files in sync: changing the workload on one side
// is meaningless unless it is mirrored on the other. See `docs/performance.md`
// for the recorded baselines and the measurement protocol — in particular, these numbers are only valid when linked against a
// release build of qortoo-ffi (use `make bench-go`).
//
// Common conditions (mirrored in Rust): LocalConnectivity in manual mode so no
// background sync interferes, single-threaded measurement, and no handler
// registered — the trampoline cost is represented by the transaction scenario.
//
// Write scenarios replace their datatype every resetEvery* operations with the
// timer stopped. Without that the reported cost depends on how many operations
// the harness happened to run: a datatype accumulates its pending push buffer
// and its server-side history, and both make later operations slower (a long
// enough run eventually exceeds the push buffer limit outright). Resetting keeps
// both halves of the pair in the same flat regime.
//
// These use the classic b.N loop rather than b.Loop because they need
// StopTimer/StartTimer to exclude the resets that the Rust side also excludes.

import (
	"fmt"
	"testing"
)

const (
	benchCollection = "bench-collection"

	// implLabel makes every benchmark a sub-benchmark named `impl=go`, so a
	// merged result file can be split with `benchstat -col /impl` to put this
	// half next to the Rust half of the same scenario. The Rust harness emits
	// `impl=rust` lines under the identical scenario names.
	implLabel = "impl=go"

	// Operations recorded per transaction in the transaction scenario.
	txOps = 10

	// Single writes performed on one datatype before it is replaced. Must match
	// RESET_EVERY_OPS in benches/qortoo_bench.rs.
	resetEveryOps = 50_000

	// Transactions performed on one datatype before it is replaced.
	resetEveryTxs = resetEveryOps / txOps

	// Push-pulls performed on one datatype before it is replaced. The server
	// keeps every pushed transaction in its history, which is what this bounds.
	resetEverySyncs = 5_000

	// Counters built per client in the build scenario, matching
	// BUDGET_BUILD_COUNTER in benches/qortoo_bench.rs. Run that benchmark with
	// `-benchtime 100x`; beyond this budget the per-build cost stops being
	// representative, so the client is recycled to at least keep the run alive.
	countersPerClient = 100
)

// benchSink keeps the compiler from eliding the result of read benchmarks.
var benchSink int64

// nextBenchKey is the source of unique datatype keys: a key can only be
// registered once per client.
var nextBenchKey int

// benchClient owns a client and the connectivity it was built with, so a
// benchmark that recycles clients can release both — each client runs its own
// tokio runtime, and leaving them open exhausts the OS thread limit.
type benchClient struct {
	client *Client
	conn   *LocalConnectivity
}

func (bc *benchClient) close() {
	bc.client.Close()
	bc.conn.Close()
}

// newBenchClient builds a client whose connectivity never syncs on its own.
func newBenchClient(b *testing.B, alias string) *benchClient {
	b.Helper()
	conn := NewLocalConnectivity()
	conn.SetRealtime(false)
	client, err := NewClient(benchCollection, alias, WithLocalConnectivity(conn))
	if err != nil {
		b.Fatalf("client must build: %v", err)
	}
	return &benchClient{client: client, conn: conn}
}

// newManagedBenchClient is newBenchClient for benchmarks that keep one client
// for their whole run.
func newManagedBenchClient(b *testing.B, alias string) *Client {
	b.Helper()
	bc := newBenchClient(b, alias)
	b.Cleanup(bc.close)
	return bc.client
}

func nextBenchCounter(b *testing.B, client *Client) *Counter {
	b.Helper()
	nextBenchKey++
	counter, err := client.CreateCounter(fmt.Sprintf("bench-%d", nextBenchKey), nil)
	if err != nil {
		b.Fatalf("counter must build: %v", err)
	}
	return counter
}

// benchWithReset runs op b.N times, replacing the datatype every resetEvery
// operations with the timer stopped.
func benchWithReset(b *testing.B, alias string, resetEvery int, op func(*Counter)) {
	b.Helper()
	client := newManagedBenchClient(b, alias)

	var counter *Counter
	b.Cleanup(func() {
		if counter != nil {
			counter.Close()
		}
	})

	b.ReportAllocs()
	b.ResetTimer()
	for i := 0; i < b.N; i++ {
		if i%resetEvery == 0 {
			b.StopTimer()
			if counter != nil {
				counter.Close()
			}
			counter = nextBenchCounter(b, client)
			b.StartTimer()
		}
		op(counter)
	}
}

// BenchmarkIncreaseBy measures the cheapest mutating operation.
func BenchmarkIncreaseBy(b *testing.B) {
	b.Run(implLabel, func(b *testing.B) {
		benchWithReset(b, "bench-increase-by", resetEveryOps, func(counter *Counter) {
			if _, err := counter.IncreaseBy(1); err != nil {
				b.Fatalf("increase must succeed: %v", err)
			}
		})
	})
}

// BenchmarkGetValue measures the cheapest call of the whole API, where the
// relative weight of the binding is largest.
func BenchmarkGetValue(b *testing.B) {
	b.Run(implLabel, func(b *testing.B) {
		client := newManagedBenchClient(b, "bench-get-value")
		counter := nextBenchCounter(b, client)
		b.Cleanup(counter.Close)
		if _, err := counter.Increase(); err != nil {
			b.Fatalf("increase must succeed: %v", err)
		}

		b.ReportAllocs()
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			benchSink = counter.Value()
		}
	})
}

// BenchmarkTransaction measures a transaction of txOps operations, which crosses
// the callback trampoline (Go → Rust → Go) that Rust does not pay.
func BenchmarkTransaction(b *testing.B) {
	b.Run(implLabel, func(b *testing.B) {
		benchWithReset(b, "bench-transaction", resetEveryTxs, func(counter *Counter) {
			err := counter.Transaction("bench", func(tx *Counter) error {
				for range txOps {
					if _, err := tx.IncreaseBy(1); err != nil {
						return err
					}
				}
				return nil
			})
			if err != nil {
				b.Fatalf("transaction must commit: %v", err)
			}
		})
	})
}

// BenchmarkSyncLocal measures one write plus a manual push-pull against the
// in-memory server: the protocol path, where the binding should be negligible.
func BenchmarkSyncLocal(b *testing.B) {
	b.Run(implLabel, func(b *testing.B) {
		benchWithReset(b, "bench-sync-local", resetEverySyncs, func(counter *Counter) {
			if _, err := counter.IncreaseBy(1); err != nil {
				b.Fatalf("increase must succeed: %v", err)
			}
			if err := counter.Sync(); err != nil {
				b.Fatalf("sync must succeed: %v", err)
			}
		})
	})
}

// BenchmarkBuildCounter measures building and closing a counter — the one-time
// cost per datatype, including this binding's runtime.AddCleanup registration.
// Run it with `-benchtime 100x` (as `make bench-go` does): the per-build cost is
// not constant, so only a fixed budget is comparable with the Rust side.
func BenchmarkBuildCounter(b *testing.B) {
	b.Run(implLabel, func(b *testing.B) {
		// Keys are unique because a key can only be registered once per client;
		// formatting them is deliberately left out of the measured region, as it
		// is on the Rust side.
		keys := make([]string, b.N)
		for i := range keys {
			keys[i] = fmt.Sprintf("build-%d", i)
		}
		client := newBenchClient(b, "bench-build-counter")
		defer func() { client.close() }()

		b.ReportAllocs()
		b.ResetTimer()
		for i := 0; i < b.N; i++ {
			if i > 0 && i%countersPerClient == 0 {
				// Recycling must release the previous client: every client owns
				// a tokio runtime, and keeping them all open exhausts OS threads.
				b.StopTimer()
				client.close()
				client = newBenchClient(b, "bench-build-counter")
				b.StartTimer()
			}
			counter, err := client.client.CreateCounter(keys[i], nil)
			if err != nil {
				b.Fatalf("counter must build: %v", err)
			}
			counter.Close()
		}
	})
}
