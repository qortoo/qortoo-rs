# Getting Started

This is the complete first-run flow: adding the crate, creating a datatype, and syncing two
clients. The root README's Quick Start is the short version of the same thing; this document
adds what that snippet leaves out — Variable, and manual synchronization between two clients.

## Add the dependency

`qortoo` is pre-release (`0.1.0`) and not published to crates.io. Depend on it directly from
the repository, pinned to a commit — the same way `qortoo-go`'s own CI consumes it:

```toml
[dependencies]
qortoo = { git = "https://github.com/qortoo/qortoo-rs", rev = "<commit-sha>" }
```

Use a full commit SHA, not a branch name; a branch moves out from under you, a commit doesn't.
See the root [README](../README.md#requirements) for the Rust version this crate declares and
the one CI actually verifies.

## Your first Counter

```rust
use qortoo::Client;

let client = Client::builder("my-collection", "my-client").build().unwrap();

let counter = client
    .create_datatype("my-counter")
    .build_counter()
    .unwrap();

counter.increase().unwrap();
counter.increase_by(5).unwrap();
assert_eq!(counter.get_value(), 6);
```

No connectivity backend was supplied, so this client got the default one: it holds no state
and shares nothing with any other client. That's expected here — this example has only one
client. See [Connectivity](connectivity.md) for what the bundled backends do and don't do.

## Your first Variable

`Variable` stores one JSON value with last-writer-wins semantics. It starts out holding JSON
`null`; any `serde`-serializable type round-trips through `set`/`get` after that:

```rust
use qortoo::Client;
use serde_json::Value;

let client = Client::builder("my-collection", "my-other-client").build().unwrap();

let name = client
    .create_datatype("my-name")
    .build_variable()
    .unwrap();
assert_eq!(name.get::<Value>().unwrap(), Value::Null);

name.set(&"ada").unwrap();
assert_eq!(name.get::<String>().unwrap(), "ada");
```

See [Variable](variable.md) for the JSON contract, timestamp precedence, and snapshot format.

## Syncing two clients

A single process can run more than one client and let them synchronize through
`LocalConnectivity`, an in-process backend built for exactly this — tests, prototyping, and
demonstrations. In manual mode, synchronization happens only when a datatype calls `sync()`:

```rust
use qortoo::{Client, Datatype, LocalConnectivity};

let connectivity = LocalConnectivity::new_arc();
connectivity.set_realtime(false); // require explicit sync() calls

let client1 = Client::builder("my-collection", "client-1")
    .with_connectivity(connectivity.clone())
    .build()
    .unwrap();
let client2 = Client::builder("my-collection", "client-2")
    .with_connectivity(connectivity)
    .build()
    .unwrap();

let counter1 = client1.create_datatype("shared-counter").build_counter().unwrap();
counter1.increase().unwrap();
counter1.sync().unwrap(); // push what client1 has

let counter2 = client2.subscribe_datatype("shared-counter").build_counter().unwrap();
counter2.sync().unwrap(); // pull what client1 pushed

assert_eq!(counter2.get_value(), 1);
```

A subscribing client must create (or already know of) the resource before it can subscribe —
`subscribe_datatype` on a key nothing has ever created fails. That's why `client1` creates and
syncs first. This exact flow is exercised as a doctest in
[`local_connectivity.rs`](../src/connectivity/local_connectivity.rs); running
`cargo test --doc local_connectivity` runs it.

## Running this yourself

```shell
git clone https://github.com/qortoo/qortoo-rs
cd qortoo-rs
cargo test --doc local_connectivity   # the two-client sync example above, verified
cargo doc --no-deps --open            # the full API reference
```

There is no `cargo run` entry point for the snippets on this page — they're library code, not
binaries. The observability examples under `## Observability` in the root README are the
crate's only runnable examples today.
