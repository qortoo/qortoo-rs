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
	cargo tarpaulin -o html -o xml -o Lcov --workspace --tests --all-features --engine Llvm --fail-under 90 --output-dir ./coverage --exclude-files 'benches/*'
	open coverage/tarpaulin-report.html

# Local, informational only — not run in CI. Line coverage is a poor completeness
# signal for a C ABI boundary crate: most of qortoo-ffi is one-line pass-through
# wrappers, and a hard percentage gate on it tends to reward filler tests (a null-check
# nobody would otherwise write, a core algorithm re-verified through a pointer) instead
# of real boundary contract coverage. The authoritative completeness bar is the
# reviewable checklist in the FFI test-plan doc: every `qortoo_*` symbol exercised,
# every null/invalid-UTF-8/ownership/error-code case asserted — not this number. Run it
# yourself if you want a rough sanity check after touching qortoo-ffi/src.
.PHONY: ffi-coverage
ffi-coverage:
	cargo tarpaulin -p qortoo-ffi --tests --all-features --engine Llvm --include-files 'qortoo-ffi/src/*' --fail-under 85

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

# ── Native C SDK ───────────────────────────────────────────────────────────────
.PHONY: ffi
ffi:
	cargo build -p qortoo-ffi

# Stages a location-independent native SDK — qortoo.h, libqortoo_ffi.a, and
# pkg-config metadata — under target/native-sdk/$(NATIVE_SDK_PROFILE). Debug and
# release land in separate directories, so consumers cannot accidentally link a
# stale artifact from the other profile.
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

# ── Rust core benchmarks (see docs/performance.md) ────────────────────────────
.PHONY: bench bench-rust
bench: bench-rust

bench-rust:
	cargo bench --bench qortoo_bench

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
