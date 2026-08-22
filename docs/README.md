# Qortoo-rs Documentation

Qortoo-rs is a Rust SDK for **conflict-free replicated data types (CRDTs)** with distributed synchronization. It provides atomic transactions with rollback, read-only observation modes, and pluggable connectivity backends.

## Origin of the Name `Qortoo`

`Qortoo` is **Quantum + Ortoo**. Qortoo is a new beginning for the [Orda project](https://github.com/orda-io). Orda is a name derived from [Yam / Ortoo](https://en.wikipedia.org/wiki/Yam_(route)), inspired by the similarity between the Mongol Empire's communication system and the synchronization functionality this project aims to implement. In fact, Ortoo was used first, but since it was already widely used elsewhere, the name Orda was chosen instead. As you can see from Qortoo's git history, this project was briefly named SyncYam, which also derives from Yam.

Quantum is inspired by [Quantum Entanglement](https://en.wikipedia.org/wiki/Quantum_entanglement). Qortoo's approach of replicating data types and rapidly synchronizing them through operations was thought to be similar to quantum entanglement. Since Ortoo was already a commonly used name, Quantum + Ortoo were combined to create Qortoo for differentiation.

This is just an unnecessarily detailed explanation of the name.

## Architecture at a Glance

Each datatype is composed of five layers stacked vertically. A user operation passes through all layers top-down:

```
┌─────────────────────────────────────────┐
│  Public API  (e.g., Counter)            │  ← User-facing type
├─────────────────────────────────────────┤
│  Transactional Layer                    │  ← Atomic scope, DeferGuard commit/rollback
├─────────────────────────────────────────┤
│  Mutable Layer                          │  ← Local CRDT state, push buffer, TxRecord
├─────────────────────────────────────────┤
│  Wired Layer                            │  ← Sync with connectivity backend
├─────────────────────────────────────────┤
│  CRDT Layer  (e.g., CounterCrdt)        │  ← Pure CRDT implementation
└─────────────────────────────────────────┘
```

## Core Architecture Documents

| Document | Description |
|----------|-------------|
| [Architecture](architecture.md) | Layer stack, shared state model, operation flow, and concurrency model |
| [Core Types](core-types.md) | UID roles, CRDT timestamps and element IDs, operation progress, and transaction sequence coordinates |
| [Client and DatatypeBuilder](client-and-datatype-builder.md) | `Client` builder pattern, collection/key naming validation, and the `DatatypeBuilder` chain |
| [Datatype State](datatype-state.md) | `DatatypeState` lifecycle, write access, sync intent states, and unsubscribe cleanup |
| [Transaction and Rollback](transaction-and-rollback.md) | `TxRecord` structure, transaction lifecycle, and inverse-operation rollback |
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
| Getting Started | TBD — Installation, basic setup, and first datatype |
| Datatypes Reference | TBD — Counter API; planned `Variable` and `Map` types |
| Testing Guide | TBD — Test macros, `LocalConnectivity` realtime pitfall, and async test patterns |

## Maintaining This Documentation

- New documents should be placed in this directory and linked in the tables above.
