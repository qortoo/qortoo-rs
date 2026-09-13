# Qortoo-rs Documentation

Qortoo-rs is a Rust SDK for **conflict-free replicated data types (CRDTs)** with atomic transactions and rollback, read-only observation modes, and a pluggable `Connectivity` trait for synchronization. The two bundled backends are in-process only — see [Connectivity](connectivity.md) for what they do and don't do.

## Origin of the Name `Qortoo`

`Qortoo` is **Quantum + Ortoo**. Qortoo is a new beginning for the [Orda project](https://github.com/orda-io). Orda is a name derived from [Yam / Ortoo](https://en.wikipedia.org/wiki/Yam_(route)), inspired by the similarity between the Mongol Empire's communication system and the synchronization functionality this project aims to implement. In fact, Ortoo was used first, but since it was already widely used elsewhere, the name Orda was chosen instead. As you can see from Qortoo's git history, this project was briefly named SyncYam, which also derives from Yam.

Quantum is inspired by [Quantum Entanglement](https://en.wikipedia.org/wiki/Quantum_entanglement). Qortoo's approach of replicating data types and rapidly synchronizing them through operations was thought to be similar to quantum entanglement. Since Ortoo was already a commonly used name, Quantum + Ortoo were combined to create Qortoo for differentiation.

This is just an unnecessarily detailed explanation of the name.

## Architecture at a Glance

Each datatype is composed from five responsibilities: the public API an application calls, a
transactional layer, the mutable state, a wired layer, and the CRDT state machine. These are
responsibilities rather than a containment chain. The transactional layer and the wired layer
both hold the same mutable state and both write to it — local operations arrive through one,
synchronization through the other — so a datatype is one state with two writers, not five
layers that wrap each other.

[Architecture](architecture.md) owns the responsibility table, the diagrams, and the concurrency model.

## Core Architecture Documents

| Document | Description |
|----------|-------------|
| [Architecture](architecture.md) | Layer stack, shared state model, operation flow, and concurrency model |
| [Core Types](core-types.md) | UID roles, CRDT timestamps and element IDs, operation progress, and transaction sequence coordinates |
| [Client and DatatypeBuilder](client-and-datatype-builder.md) | `Client` builder pattern, collection/key naming validation, and the `DatatypeBuilder` chain |
| [Datatype State](datatype-state.md) | `DatatypeState` lifecycle, write access, sync intent states, and unsubscribe cleanup |
| [Transaction and Rollback](transaction-and-rollback.md) | `TxRecord` structure, transaction lifecycle, and inverse-operation rollback |
| [Counter](counter.md) | Cumulative `i64` arithmetic, wrapping semantics, and rollback by recorded inverse |
| [Variable](variable.md) | LWW `Variable` datatype: JSON value contract, timestamp precedence, exact-restore rollback, and snapshot format |
| [Connectivity](connectivity.md) | `Connectivity` trait, `NullConnectivity`, `LocalConnectivity`/`LocalDatatypeServer`, and `WiredInterceptor` |
| [Handler System](handler-system.md) | `DatatypeHandler`, `HandlersManager`, priority-based async dispatch |
| [Event Loop](event-loop.md) | Priority-based event processing, channel types, and exponential backoff behavior |
| [Error Handling](error-handling.md) | Error taxonomy, `RecoveryAction` routing, and sync-path vs commit-path recovery |
| [Observability](observability.md) | Rust tracing, log layer, Prometheus metrics, Pyroscope profiling, and managed exporters |
| [C ABI and Native SDK](go-binding.md) | `qortoo-ffi` ownership, error/callback contracts, header generation, and SDK staging |
| [Performance](performance.md) | Rust core fixed-budget benchmark harness and comparison contract |

## Usage Guides

| Document | Description |
|----------|-------------|
| [Getting Started](getting-started.md) | Adding the crate, a first Counter and Variable, and manual two-client sync |

## Maintaining This Documentation

- New documents should be placed in this directory and linked in the tables above.
