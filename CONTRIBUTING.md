# Contributing

The build, test, and documentation procedure below runs with only `cargo`, `make`, and
`git` — nothing here depends on Claude Code or any of its plugins. `AGENTS.md` adds
work rules on top of this (test naming, what belongs in `qortoo-ffi` vs the core crate,
plan-tracking conventions) that apply to every contributor, agent or human, regardless
of tooling.

## Prerequisites

- Edition 2024, with a declared minimum of Rust `1.87.0` (`rust-version` in
  `Cargo.toml`). CI builds and tests only against `1.97.1`; treat versions in between as
  unverified. See the root [README](README.md#requirements).
- Nightly Rust, for the pinned `rustfmt` `make lint` runs (`cargo +nightly-2026-08-02
  fmt`).
- `cargo-tarpaulin`, installed by `make install`, for coverage.

## Build and Test

```shell
make install                          # install cargo-tarpaulin

cargo test                            # run all tests
cargo test --all-features             # include observability-gated tests
cargo test test_name                  # run a single test
cargo test module_name::              # run a module's tests

make lint                             # nightly fmt check, cargo check, clippy -D warnings
make tarpaulin                        # coverage (local: 90% minimum; CI: 80% minimum)
make doc                              # generate rustdoc
make bench                            # fixed-budget benchmarks (see docs/performance.md)
make native-sdk-stage                 # stage the C header, static lib, pkg-config, manifest

make obs-up                           # start the local Grafana/Prometheus/Tempo/Loki/Pyroscope stack
make obs-down                         # stop it
make obs-down-v                       # stop it and remove persisted volumes
make obs-logs                         # tail its container logs
```

`cargo test --all-features` exercises `observability-trace`/`observability-metrics` code
paths; it does not require the local stack to be running for the tests themselves to
pass, but the stack lets you inspect what those code paths actually produced. See the
root README's `## Observability` section for the runnable trace/log/metrics/profile
examples, and [`docs/observability.md`](docs/observability.md) for the full reference.

## Documentation

`docs/` holds Qortoo's concept documents — the contracts and design rationale behind
each part of the system, cross-linked with rustdoc rather than duplicating it.
[`docs/README.md`](docs/README.md) is the index; read it to find where a given topic
lives before adding a new document.

Every concept doc in `docs/` follows the same shape: `Overview` (in the untitled lead
paragraph) → `Model` → `Rules and Guarantees` → `Behavior` → `Rationale` → `Code Map` →
`Related Concepts`. Start a new one by copying the structure of an existing doc closest
in kind to what you're adding (a datatype doc like [`docs/counter.md`](docs/counter.md),
a mechanism doc like [`docs/event-loop.md`](docs/event-loop.md)) rather than starting
from a blank page. A `Code Map` entry that names a test is only honest if that test
still exists and still verifies the claim next to it — check before you cite one.

A concept doc describes the system as it is now and the rule that holds it in place:
write principles, not development history. Qortoo has not shipped a release yet, so
`docs/` should describe only current, intended behavior — don't add migration notes or
"this used to work differently" asides for unreleased changes.

Verify a documentation change the same way `make lint`-adjacent CI does:

```shell
cargo doc --no-deps                                                          # default features
RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps        # fail on broken intra-doc links
cargo doc --no-deps --all-features
RUSTDOCFLAGS="-D rustdoc::broken_intra_doc_links" cargo doc --no-deps --all-features
```

Check every relative Markdown link, in-file anchor, and source-path reference you add or
touch actually resolves:

```shell
make check-docs   # or: python3 scripts/check_doc_links.py
```

This also flags a new `docs/*.md` file that isn't linked from
[`docs/README.md`](docs/README.md)'s index. It never touches the network — external
URLs are out of scope for this check — and CI runs it on every change to a Markdown
file or to the script itself, independent of the main build/test workflow.

If you add or change a Mermaid diagram, confirm it still renders — a syntax error in
one is otherwise silent until someone views the page:

```shell
make check-mermaid   # or: python3 scripts/check_mermaid.py
```

Unlike `check-docs`, this needs Node/npm and fetches `@mermaid-js/mermaid-cli` from the
npm registry on first run, so it's a separate target and a separate CI job. Neither
script checks that a diagram's labels still match the code — that's still a manual
comparison against `src/`, same as any other claim in a concept doc.

## Coding Style

- Standard Rust style: 4-space indentation, `snake_case` for functions and modules,
  `PascalCase` for types.
- Formatting follows `.rustfmt.toml` (`group_imports = "StdExternalCrate"`,
  `imports_granularity = "Crate"`) via the pinned nightly `rustfmt` above.
- Clippy warnings are treated as errors (`make lint`).

## Commits and Pull Requests

- Commit subjects follow an emoji + conventional-commits shape:
  `🧬feat(scope): summary`, `🪲fix(scope): summary`, `📜docs(scope): summary`,
  `💿CI/CD(scope): summary`, `💄refactor(scope): summary`, `🧩chore(scope): summary`,
  `🧪test(scope): summary`, `⚡perf(scope): summary`. Keep `scope` specific to the
  module touched (`datatypes`, `client`, `connectivity`, `workflow`, …).
- A PR description should summarize the change, list the commands you ran to verify it,
  and link any related issue.
- If behavior changes, update the concept doc that owns it and add or adjust tests in
  the same change — don't let `docs/` drift from the code.
