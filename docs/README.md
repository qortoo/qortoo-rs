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
| [Transaction and Rollback](transaction-and-rollback.md) | `TxRecord` structure, transaction lifecycle, and inverse-operation rollback |
| [Event Loop](event-loop.md) | Priority-based event processing, channel types, and exponential backoff behavior |
| [Observability](observability.md) | Tracing, log layer, Prometheus metrics, and Pyroscope profiling integration |

## Usage Guides

| Document | Description |
|----------|-------------|
| Getting Started | TBD — Installation, basic setup, and first datatype |
| Client and DatatypeBuilder | TBD — `Client` builder pattern, collection scoping, and datatype registration |
| Datatypes Reference | TBD — Counter API; planned `Variable` and `Map` types |
| Connectivity Backends | TBD — `NullConnectivity`, `LocalConnectivity`, and implementing a custom backend |
| Error Handling | TBD — Error types, `DatatypeErrorWithActions`, `EventLoopAction`, and recovery strategies |
| Handler System | TBD — `DatatypeHandler`, `HandlersManager`, priority-based callback dispatch |
| Testing Guide | TBD — Test macros, `LocalConnectivity` realtime pitfall, and async test patterns |

## Maintaining This Documentation

- New documents should be placed in this directory and linked in the tables above.
