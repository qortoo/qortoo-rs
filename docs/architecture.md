# Qortoo-rs Architecture

## Overview

Qortoo-rs is a Rust SDK for CRDTs (Conflict-free Replicated Data Types) with distributed synchronization. All datatype instances are thread-safe and support atomic transactions with rollback.

## Datatype Layer Stack

Each datatype is composed of five layers stacked vertically. A user operation passes through all layers top-down.

```
┌────────────────────────────────────────────────────┐
│           Public API  (e.g., Counter)              │  ← User-facing type
│           implements DatatypeBlanket               │
├────────────────────────────────────────────────────┤
│           Transactional Layer                      │  ← Transaction scope,
│           TransactionalDatatype                    │     DeferGuard commit/rollback
├────────────────────────────────────────────────────┤
│           Mutable Layer                            │  ← Local state: CRDT, op_id,
│           MutableDatatype                          │     push_buffer, tx_record
├────────────────────────────────────────────────────┤
│           Wired Layer                              │  ← Push/pull sync with
│           WiredDatatype                            │     connectivity backend
├────────────────────────────────────────────────────┤
│           CRDT Layer   (e.g., CounterCrdt)         │  ← Pure CRDT: execute_local_operation,
│           Crdt enum                                │     execute_remote_operation,
│                                                    │     execute_inverse_operation
└────────────────────────────────────────────────────┘
```

> For event loop internals (channel architecture, BackOff, Notify flow) see [`docs/event-loop.md`](event-loop.md).

### Layer Responsibilities

| Layer | Struct | Key Responsibility |
|-------|--------|--------------------|
| Public API | `Counter`, etc. | User-facing methods; implements `DatatypeBlanket` |
| Transactional | `TransactionalDatatype` | Transaction scope via `TransactionContext` and `DeferGuard`; serializes concurrent ops via `op_mutex` / `tx_mutex` |
| Mutable | `MutableDatatype` | Owns `Crdt`, `OperationId`, `PushBuffer`, `TxRecord`; executes and records operations |
| Wired | `WiredDatatype` | Assembles `PushPullPack` and calls `Connectivity::push_pull`; drives the event loop |
| CRDT | `CounterCrdt`, … | Pure state machine; no I/O, no locking |

## Shared State Model

```
Arc<TransactionalDatatype>
 ├── attr: Arc<Attribute>          ← immutable config (cuid, type, option, handlers)
 ├── mutable: Arc<RwLock<MutableDatatype>>
 │    ├── crdt: Crdt               ← CRDT state
 │    ├── op_id: OperationId       ← lamport + cseq counter
 │    ├── push_buffer              ← committed-but-not-acked transactions
 │    ├── tx_record: TxRecord      ← pending transaction + rollback save point
 │    └── state: DatatypeState     ← lifecycle state
 └── (WiredDatatype wraps the same Arc<RwLock<MutableDatatype>>)
```

`Arc<Attribute>` is shared across all layers and is the best place for per-datatype cross-cutting concerns (handler registry, push buffer options, etc.).

## Operation Flow

### Local write

```
User calls counter.increase(1)
  │
  ▼
TransactionalDatatype::execute_local_operation_as_tx()
  ├── acquire op_mutex
  ├── begin_transaction_if_needed()   ← creates TransactionContext + DeferGuard
  │
  ▼
MutableDatatype::execute_local_operation()
  ├── op.set_lamport(op_id.lamport + 1)
  ├── crdt.execute_local_operation(&op)   ─── succeeds?
  │     YES → tx_record.record_operation()  ← append op + update rollback save point
  │           op_id.next(is_new_tx)         ← advance lamport (and cseq if new tx)
  │     NO  → return Err (op_id unchanged)
  │
  ▼
DeferGuard drop → end_transaction(committed=true)
  └── push_buffer.enqueue(tx)   ← ready to sync
```

### Sync (push/pull)

```
EventLoop fires PushTransaction event
  │
  ▼
WiredDatatype::push_pull()
  ├── mutable.read() → assemble PushPullPack (push_buffer contents)
  ├── connectivity.push_pull(&pack)
  ├── mutable.write() → apply pulled transactions
  │    ├── execute_remote_transaction() for each remote tx
  │    └── push_buffer.deque(acked_cseq)
  └── set_state(pulled.state)
```

## Concurrency Model

- `mutable: Arc<RwLock<MutableDatatype>>` — all CRDT mutation is serialized here
- `op_mutex: NoGuardMutex` — serializes concurrent `execute_local_operation` calls
- `tx_mutex: NoGuardMutex` — serializes concurrent transaction scopes
- Handler notifications are dispatched via `rt_handle.spawn` **after** the write lock is released to avoid deadlock (handlers may call `get_value()` which takes a read lock)

## Key Types Quick Reference

| Type | Location | Purpose |
|------|----------|---------|
| `OperationId` | `src/types/operation_id.rs` | `(lamport, cuid, cseq)` — identifies an operation's position |
| `Operation` | `src/operations/mod.rs` | Single CRDT operation with `OperationBody` and `lamport` |
| `Transaction` | `src/operations/transaction.rs` | Ordered group of operations sharing `cuid`/`cseq` |
| `TxRecord` | `src/datatypes/tx_record.rs` | Pending transaction buffer + rollback save point |
| `PushPullPack` | `src/types/push_pull_pack.rs` | Wire format for push/pull exchange |
| `Attribute` | `src/datatypes/common.rs` | Immutable per-datatype config shared across layers |
| `DatatypeState` | `src/types/datatype.rs` | Lifecycle state machine |
