.PHONY: install
install:
	cargo install cargo-tarpaulin --version 0.37.0 --locked

.PHONY: lint
lint:
	cargo +nightly-2026-08-02 fmt --all --check
	cargo check --all-features --tests
	cargo clippy --workspace --all-targets --tests --all-features -- -D warnings

.PHONY: tarpaulin
tarpaulin:
	-cargo tarpaulin -o html -o xml -o Lcov --workspace --tests --all-features --engine Llvm --fail-under 90 --output-dir ./coverage --exclude-files 'benches/*'
	open coverage/tarpaulin-report.html

.PHONY: doc
doc:
	cargo doc --no-deps --open

# ── FFI ABI contract (qortoo-ffi/src/version.rs) ────────────────────────────────
# The generated header is checked in so a diff shows up in code review, not just in
# CI: cbindgen regenerates it as a build.rs side effect of `cargo build`, so if the
# committed file and the source disagree, this fails with a non-empty diff.
.PHONY: ffi-header-check
ffi-header-check:
	cargo build -p qortoo-ffi
	@git diff --exit-code -- qortoo-ffi/include/qortoo.h || { \
		echo "error: qortoo-ffi/include/qortoo.h is stale; commit the regenerated header" >&2; \
		exit 1; \
	}

# Every exported qortoo_* symbol is an ABI commitment. This diffs the actual
# defined global symbols in the built static library against the checked-in
# allowlist; an unexpected addition or removal means the ABI changed and
# QORTOO_ABI_VERSION_MAJOR/MINOR (qortoo-ffi/src/version.rs) must move with it.
# macOS (Mach-O) prefixes every global symbol with `_`; Linux (ELF) does not —
# the sub() strips it so both platforms compare against the same list.
.PHONY: abi-symbols-check
abi-symbols-check:
	cargo build -p qortoo-ffi
	@actual=$$(mktemp); \
	nm -g target/debug/libqortoo_ffi.a 2>/dev/null \
		| awk '$$2 == "T" && $$3 ~ /^_?qortoo_/ { sym = $$3; sub(/^_/, "", sym); print sym }' \
		| sort -u > $$actual; \
	if ! diff -u qortoo-ffi/abi-symbols.txt $$actual; then \
		echo "error: exported qortoo_* symbols changed — update qortoo-ffi/abi-symbols.txt" >&2; \
		echo "       and bump QORTOO_ABI_VERSION_MAJOR/MINOR in qortoo-ffi/src/version.rs" >&2; \
		rm -f $$actual; \
		exit 1; \
	fi; \
	rm -f $$actual

# ── Go binding (go/qortoo, linked against qortoo-ffi) ────────────────────────────
.PHONY: ffi
ffi:
	cargo build -p qortoo-ffi

# Stages a location-independent native SDK — qortoo.h, libqortoo_ffi.a, and
# pkg-config metadata — under target/native-sdk/$(NATIVE_SDK_PROFILE). The Go
# binding carries no default include/library path (see go/qortoo/cgo.go): every
# target below points CGO_CFLAGS/CGO_LDFLAGS at this staged directory, never at a
# fixed relative path. Debug and release land in separate directories, so there is
# no shared location where the wrong one could get linked by mistake.
NATIVE_SDK_PROFILE ?= debug
NATIVE_SDK_DIR := target/native-sdk/$(NATIVE_SDK_PROFILE)
QORTOO_FFI_VERSION := $(shell grep -m1 '^version' qortoo-ffi/Cargo.toml | cut -d'"' -f2)
RUST_TARGET := $(shell rustc -vV | sed -n 's/^host: //p')

.PHONY: native-sdk-stage
native-sdk-stage:
	cargo build -p qortoo-ffi $(if $(filter release,$(NATIVE_SDK_PROFILE)),--release)
	rm -rf $(NATIVE_SDK_DIR)
	mkdir -p $(NATIVE_SDK_DIR)/include $(NATIVE_SDK_DIR)/lib/pkgconfig
	cp qortoo-ffi/include/qortoo.h $(NATIVE_SDK_DIR)/include/
	cp target/$(NATIVE_SDK_PROFILE)/libqortoo_ffi.a $(NATIVE_SDK_DIR)/lib/
	sed 's/@VERSION@/$(QORTOO_FFI_VERSION)/' qortoo-ffi/qortoo-ffi.pc.in > $(NATIVE_SDK_DIR)/lib/pkgconfig/qortoo-ffi.pc
	@abi_major=$$(grep '#define QORTOO_ABI_VERSION_MAJOR' qortoo-ffi/include/qortoo.h | awk '{print $$NF}'); \
	abi_minor=$$(grep '#define QORTOO_ABI_VERSION_MINOR' qortoo-ffi/include/qortoo.h | awk '{print $$NF}'); \
	printf '{\n  "sdk_version": "%s",\n  "abi_version_major": %s,\n  "abi_version_minor": %s,\n  "profile": "%s",\n  "target": "%s"\n}\n' \
		"$(QORTOO_FFI_VERSION)" "$$abi_major" "$$abi_minor" "$(NATIVE_SDK_PROFILE)" "$(RUST_TARGET)" \
		> $(NATIVE_SDK_DIR)/manifest.json

.PHONY: go-test
go-test: export CGO_CFLAGS := -I$(CURDIR)/target/native-sdk/debug/include
go-test: export CGO_LDFLAGS := -L$(CURDIR)/target/native-sdk/debug/lib
go-test: native-sdk-stage
	cd go/qortoo && go vet ./... && go test -race ./...

# ── Benchmarks (Rust core vs. Go binding; see docs/performance.md) ─────────────
# Both sides run a pinned iteration budget per scenario: per-operation cost in
# this SDK grows with accumulated state, so the halves are only comparable when
# they run the same number of operations. Keep the budgets below in sync with
# the BUDGET_* constants of benches/qortoo_bench.rs.
.PHONY: bench-rust
bench-rust:
	cargo bench --bench qortoo_bench

# Go benchmarks must link the RELEASE qortoo-ffi. native-sdk-stage keeps
# debug/release in separate directories (target/native-sdk/{debug,release}), so
# there is no shared location where a debug artifact could get linked instead.
.PHONY: bench-go
bench-go: export CGO_CFLAGS := -I$(CURDIR)/target/native-sdk/release/include
bench-go: export CGO_LDFLAGS := -L$(CURDIR)/target/native-sdk/release/lib
bench-go:
	$(MAKE) native-sdk-stage NATIVE_SDK_PROFILE=release
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
