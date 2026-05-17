# Transaction and Rollback

## Overview

Every local write in Qortoo-rs is atomic. If any operation in a transaction fails, all previously applied operations are undone via **inverse operations** — no CRDT clone is kept.

The central struct is `TxRecord` (`src/datatypes/tx_record.rs`), which lives inside `MutableDatatype` and manages:
1. The **pending transaction buffer** — operations applied but not yet committed
2. The **rollback save point** — the `OperationId` and `DatatypeState` to restore on failure

## TxRecord Structure

```rust
pub struct TxRecord {
    pub pending: Option<Transaction>,   // None = no active transaction
    pub rollback_op_id: OperationId,    // op_id before the transaction started
    pub rollback_state: DatatypeState,  // state before the transaction started
}
```

- `pending` is `None` when idle, `Some(tx)` while a transaction is in progress.
- `rollback_op_id` and `rollback_state` are captured at the moment the **first operation of a new transaction** is recorded (`record_operation`). They are not updated for subsequent operations in the same transaction.

## Transaction Lifecycle

```mermaid
flowchart TD
    Idle["Idle\n(pending = None)"]
    RecordFirst["record_operation()\n──────────────────────────────────────\npending = Some(Transaction::new(cuid, cseq+1))\nrollback_op_id = current op_id  ← save point set HERE\nrollback_state = current state  ← save point set HERE\nop appended to pending.operations"]
    Advance1["op_id.next(is_new_tx=true)\n→ cseq += 1, lamport += 1"]
    RecordMore["record_operation() (is_new = false)\nop appended to pending.operations"]
    Advance2["op_id.next(is_new_tx=false)\n→ lamport += 1  (cseq unchanged within same tx)"]
    Commit["end_transaction(committed=true)\npending.take() → push_buffer.enqueue(tx)\npending = None\n(rollback_op_id / rollback_state remain stale — harmless)"]
    Rollback["end_transaction(committed=false) → do_rollback()\npending.take() → iter().rev() → execute_inverse_operation()\nop_id = rollback_op_id\nset_state(rollback_state)\npending = None"]
    IdleEnd["Idle"]

    Idle -->|"first execute_local_operation() succeeds"| RecordFirst
    RecordFirst --> Advance1
    Advance1 -->|"more operations succeed"| RecordMore
    RecordMore --> Advance2
    Advance2 -->|"committed=true"| Commit
    Advance2 -->|"committed=false"| Rollback
    Commit --> IdleEnd
    Rollback --> IdleEnd
```

## Success-Only Advance Pattern

`op_id` is advanced **only after a successful operation**. This eliminates the need for a "revert" path on failure:

```rust
// MutableDatatype::execute_local_operation
op.set_lamport(self.op_id.lamport + 1);          // compute, do not advance yet
let result = self.crdt.execute_local_operation(&op);
if result.is_ok() {
    let is_new_tx = self.tx_record.record_operation(&self.op_id, self.state, op);
    self.op_id.next(is_new_tx);                  // advance only on success
}
```

On failure: `op_id` is untouched, `pending` is unchanged. The next operation retries the same lamport slot.

## Rollback via Inverse Operations

Instead of keeping a shadow clone of the CRDT, rollback applies the **inverse of each operation in reverse order**:

```mermaid
flowchart LR
    subgraph applied["Applied (in order)"]
        direction LR
        A[op_A] --> B[op_B] --> C[op_C]
    end
    subgraph rollback["Rollback (reversed)"]
        direction LR
        D["inverse(op_C)"] --> E["inverse(op_B)"] --> F["inverse(op_A)"]
    end
    applied -.->|rollback| rollback
```

Each CRDT operation must implement `execute_inverse_operation`. For `CounterIncrease(delta)`, the inverse is `increase_by(-delta)`.

```rust
// MutableDatatype::do_rollback
if let Some(tx) = self.tx_record.pending.take() {
    for op in tx.iter().rev() {
        self.crdt.execute_inverse_operation(op);
    }
    self.op_id = self.tx_record.rollback_op_id.clone();
    self.set_state(self.tx_record.rollback_state);
}
```

If `pending` is `None` (no op was ever applied), `do_rollback` is a no-op — nothing to restore.

## TransactionContext and DeferGuard

At the `TransactionalDatatype` level, a transaction is scoped by `TransactionContext` and `DeferGuard`:

```mermaid
flowchart TD
    Entry["execute_local_operation_as_tx(tx_ctx, op)"]
    Begin["begin_transaction(tx_ctx)"]
    BeginTx["BeginTx(DeferGuard)\nnew transaction scope opened"]
    SameCtx["SameCtx\njoined into caller's ongoing transaction"]
    OtherCtx["OtherCtx\nreturn error immediately"]
    Execute["MutableDatatype::execute_local_operation(op)"]
    Drop["DeferGuard::drop()\n→ end_transaction(committed = result.is_ok())"]

    Entry --> Begin
    Begin -->|BeginTx| BeginTx
    Begin -->|SameCtx| SameCtx
    Begin -->|OtherCtx| OtherCtx
    BeginTx --> Execute
    SameCtx --> Execute
    Execute --> Drop
```

- `SameCtx` — op is joined into the caller's ongoing transaction (no new `DeferGuard`).
- `OtherCtx` — another transaction is in progress; the call returns an error immediately.
- `BeginTx(DeferGuard)` — a new transaction scope is opened; `DeferGuard` commits or rolls back on drop.

## OperationId Semantics

`OperationId = (lamport: u64, cuid: Cuid, cseq: u64)`

| Field | Meaning | Advances when |
|-------|---------|---------------|
| `lamport` | Logical clock | Every successful local operation |
| `cuid` | Client unique ID | Never (set at construction) |
| `cseq` | Transaction sequence number | Each new transaction committed |

`rollback_op_id` captures the `(lamport, cuid, cseq)` snapshot from before the transaction. Restoring it on rollback returns the clock to exactly the pre-transaction state.

## Adding a New CRDT Operation Type

When adding a new `OperationBody` variant, two implementations are required:

1. `execute_local_operation` — apply the operation to the CRDT state
2. `execute_inverse_operation` — undo the operation (used by rollback)

Both must be implemented at the concrete CRDT level (e.g., `CounterCrdt`) and dispatched through the `Crdt` enum wrapper in `src/datatypes/crdts/mod.rs`.
