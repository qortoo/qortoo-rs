.PHONY: install
install:
	cargo install cargo-tarpaulin

.PHONY: lint
lint:
	cargo +nightly fmt --all --check
	cargo check --all-features --tests
	cargo clippy --workspace --all-targets --tests --all-features -- -D warnings

.PHONY: tarpaulin
tarpaulin:
	-cargo tarpaulin -o html -o xml -o Lcov --workspace --tests --all-features --engine Llvm --fail-under 90 --output-dir ./coverage --exclude-files 'benches/*'
	open coverage/tarpaulin-report.html

.PHONY: doc
doc:
	cargo doc --no-deps --open

# ── Go binding (go/qortoo, linked against qortoo-ffi) ────────────────────────────
.PHONY: ffi
ffi:
	cargo build -p qortoo-ffi

.PHONY: go-test
go-test: ffi
	cd go/qortoo && go vet ./... && go test -race ./...

# ── Benchmarks (Rust core vs. Go binding; see docs/performance.md) ─────────────
# Both sides run a pinned iteration budget per scenario: per-operation cost in
# this SDK grows with accumulated state, so the halves are only comparable when
# they run the same number of operations. Keep the budgets below in sync with
# the BUDGET_* constants of benches/qortoo_bench.rs.
.PHONY: bench-rust
bench-rust:
	cargo bench --bench qortoo_bench

# Go benchmarks must link the RELEASE qortoo-ffi: cgo.go lists target/debug
# before target/release, so a leftover debug artifact would silently be linked
# and make every number meaningless. Remove it first (restore with `make ffi`),
# then refuse to run if any debug artifact is still there.
.PHONY: bench-go
bench-go:
	rm -f target/debug/libqortoo_ffi.a target/debug/libqortoo_ffi.dylib target/debug/libqortoo_ffi.so
	cargo build -p qortoo-ffi --release
	@for lib in target/debug/libqortoo_ffi.a target/debug/libqortoo_ffi.dylib target/debug/libqortoo_ffi.so; do \
		if [ -e "$$lib" ]; then \
			echo "error: $$lib still exists; Go would link the debug build instead of the release build" >&2; \
			exit 1; \
		fi; \
	done
	cd go/qortoo && go test -run '^$$' -bench '^BenchmarkIncreaseBy$$' -benchmem -benchtime 200000x -count 10 ./...
	cd go/qortoo && go test -run '^$$' -bench '^BenchmarkGetValue$$' -benchmem -benchtime 2000000x -count 10 ./...
	cd go/qortoo && go test -run '^$$' -bench '^BenchmarkTransaction$$' -benchmem -benchtime 20000x -count 10 ./...
	cd go/qortoo && go test -run '^$$' -bench '^BenchmarkSyncLocal$$' -benchmem -benchtime 20000x -count 10 ./...
	cd go/qortoo && go test -run '^$$' -bench '^BenchmarkBuildCounter$$' -benchmem -benchtime 100x -count 10 ./...

.PHONY: bench
bench: bench-rust bench-go

# ── Benchmark tracking (benchstat; see docs/performance.md) ────────────────────
# Both halves emit the Go benchmark format under the same scenario names, one
# tagged impl=rust and one impl=go, so a single result file carries the whole
# pair and `benchstat -col /impl` renders the binding overhead with p-values.
BENCH_RESULTS := benches/results
# Machine identity, derived from the host name and sanitised to a slug. It names
# both the saved result file and the Bencher testbed, because a benchmark number
# only means anything relative to other runs on the same machine. Override with
# BENCH_HOST=… to keep history on one testbed after a machine is renamed.
BENCH_HOST ?= $(shell hostname -s | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9-' '-' | sed 's/-*$$//')
BENCH_FILE := $(BENCH_RESULTS)/$(shell date +%Y-%m-%d)-$(shell git rev-parse --short HEAD)-$(BENCH_HOST).txt
# pkg and cpu differ between the two halves (only the Go side reports them);
# ignoring them keeps both in one table instead of splitting it in two.
BENCHSTAT_IGNORE := -ignore pkg,cpu

# The sed drops Go's GOMAXPROCS suffix (`/impl=go-12`): benchstat treats it as
# part of the name, which would put the Go half on its own row instead of next
# to the Rust one. Everything measured here is single-threaded anyway.
#
# Two safeguards, both learned the hard way. The recursive make is spelled
# `$${MAKE}` rather than `$(MAKE)`: make runs any line mentioning `$(MAKE)` even
# under `-n`, which turns a dry run into a real one that truncates the result
# file through the redirect. And the output lands in a temp file that only
# replaces the saved run once it is known to contain measurements, so a crashed
# or empty run cannot destroy a good baseline.
.PHONY: bench-save
bench-save:
	@mkdir -p $(BENCH_RESULTS)
	@tmp=$$(mktemp) && \
	$${MAKE:-make} bench | sed -E 's#(/impl=go)-[0-9]+#\1#' > $$tmp; \
	if grep -q '^Benchmark' $$tmp; then \
		mv $$tmp $(BENCH_FILE); \
		echo "saved: $(BENCH_FILE)"; \
		benchstat -col /impl $(BENCHSTAT_IGNORE) $(BENCH_FILE); \
	else \
		rm -f $$tmp; \
		echo "error: the run produced no benchmark results; $(BENCH_FILE) left untouched" >&2; \
		exit 1; \
	fi

# Binding overhead of one result file. FILE defaults to the newest saved run.
.PHONY: bench-overhead
bench-overhead:
	benchstat -col /impl $(BENCHSTAT_IGNORE) $(or $(FILE),$(shell ls -t $(BENCH_RESULTS)/*.txt | head -1))

# A/B comparison across time: make bench-compare BASE=benches/results/<old>.txt
# NEW defaults to the newest saved run. Only compare runs from the same machine.
.PHONY: bench-compare
bench-compare:
	@test -n "$(BASE)" || { echo "usage: make bench-compare BASE=<file> [NEW=<file>]" >&2; exit 1; }
	benchstat $(BENCHSTAT_IGNORE) $(BASE) $(or $(NEW),$(shell ls -t $(BENCH_RESULTS)/*.txt | head -1))

# Uploads a saved run to Bencher (https://bencher.dev) for continuous tracking.
# Both halves go up in one report: the go_bench adapter reads the merged file,
# and impl=rust / impl=go keep them apart as distinct benchmarks.
#
# The key is read from the environment and must never be written into this file
# or any tracked file. Testbed identifies the machine, mirroring the rule that
# only runs from the same machine are comparable:
#
#	BENCHER_API_KEY=$$(cat ~/.bencher_token) make bench-upload
#	make bench-save && BENCHER_API_KEY=... make bench-upload   # measure, then upload
BENCHER_PROJECT ?= qortoo-sync
BENCHER_TESTBED ?= $(BENCH_HOST)

# Credentials come in two flavours and the CLI refuses the wrong pairing: an API
# key (`bencher_…`) must travel in BENCHER_API_KEY, a JWT in BENCHER_API_TOKEN.
# A key left over in BENCHER_API_TOKEN fails the run even when BENCHER_API_KEY is
# also set correctly, so route whichever one is present and clear the other.
.PHONY: bench-upload
bench-upload:
	@credential="$${BENCHER_API_KEY:-$$BENCHER_API_TOKEN}"; \
	test -n "$$credential" || { \
		echo "error: set BENCHER_API_KEY to a project key from bencher.dev" >&2; \
		exit 1; \
	}; \
	case "$$credential" in \
	bencher_*) export BENCHER_API_KEY="$$credential"; unset BENCHER_API_TOKEN ;; \
	*)         export BENCHER_API_TOKEN="$$credential"; unset BENCHER_API_KEY ;; \
	esac; \
	bencher run \
		--project $(BENCHER_PROJECT) \
		--testbed $(BENCHER_TESTBED) \
		--adapter go_bench \
		--file $(or $(FILE),$(shell ls -t $(BENCH_RESULTS)/*.txt | head -1))

# ── Observability stack (Prometheus / Grafana / Tempo / Loki / Pyroscope) ───────
.PHONY: obs-up
obs-up:
	docker compose -f qortoo-rs-docker/docker-compose.yml up -d

.PHONY: obs-down
obs-down:
	docker compose -f qortoo-rs-docker/docker-compose.yml down

.PHONY: obs-down-v
obs-down-v:
	docker compose -f qortoo-rs-docker/docker-compose.yml down -v

.PHONY: obs-logs
obs-logs:
	docker compose -f qortoo-rs-docker/docker-compose.yml logs -f
