# Counter

A counter is a conflict-free datatype holding one `i64` that changes only by adding a delta.
Every increment is an operation, not an assignment, so increments made concurrently on
different replicas all survive rather than one overwriting another — every replica ends up
holding the sum of every increment ever applied to it. Everything a counter shares with other
datatypes — transaction scope, synchronization, handlers — is defined in
[`docs/architecture.md`](architecture.md) and
[`docs/transaction-and-rollback.md`](transaction-and-rollback.md). This document owns only
what is specific to a counter: its arithmetic and what that arithmetic implies for rollback.

## Model

| Term | Meaning |
|------|---------|
| Value | The counter's current `i64`, starting at `0` |
| Delta | The signed amount an increment adds; negative deltas decrease the value |
| Wrapping arithmetic | Addition performed modulo 2^64, so the value never panics or saturates |
| Self-inverse extreme | The one delta (`i64::MIN`) whose wrapping negation is itself, not its true negative |

## Rules and Guarantees

**Cumulative.** [`increase_by(delta)`](../src/datatypes/counter.rs) adds `delta` to the current
value and returns the result; [`increase()`](../src/datatypes/counter.rs) is `increase_by(1)`. A
remote increment is applied the same way a local one is — as an addition, not a replacement —
so the order two replicas apply a set of increments in does not change the total they converge
to. Whether and when that convergence actually happens depends on the connectivity backend in
use; see [`docs/architecture.md`](architecture.md#rules-and-guarantees) for that limit.

**Negative deltas decrease.** `increase_by` takes any `i64`, including negative values; there is
no separate decrement method.

**Arithmetic wraps, it never panics or saturates.** Every addition — local, remote, and rollback
— is `i64::wrapping_add`. Increasing `i64::MAX` by `1` produces `i64::MIN`, not an error and not
a value pinned at the boundary. Wrapping is what keeps rollback exact everywhere in the next
rule: a saturating or checked counter would make some increments non-invertible right at the
boundary, and a CRDT operation that behaves differently depending on which replica applies it —
succeeding on one, saturating on another — cannot converge.

**Rollback is the wrapping negation of the delta, and it is exact even at the extreme.** Undoing
an increment adds `delta.wrapping_neg()`. For every delta except `i64::MIN` this is the
familiar negative. `i64::MIN` has no representable positive counterpart, so its wrapping
negation is `i64::MIN` itself — applying it once looks like repeating the original increment,
not undoing it. It still cancels out: modulo 2^64, `i64::MIN + i64::MIN` is congruent to `0`, so
applying the same self-inverse delta twice returns the value to exactly where it started. See
[`docs/transaction-and-rollback.md`](transaction-and-rollback.md) for the general rollback
mechanism this relies on — rollback here is by recorded inverse, not by restoring a captured
value the way [`Variable`](variable.md) does.

## Behavior

```rust
# use qortoo::{Client, Counter};
let client = Client::builder("doc-example", "counter-behavior").build().unwrap();
let counter = client.create_datatype("balance").build_counter().unwrap();

assert_eq!(counter.increase_by(5).unwrap(), 5);
assert_eq!(counter.increase_by(-2).unwrap(), 3); // negative delta decreases

counter.transaction("adjust", |c| {
    c.increase_by(10)?;
    c.increase_by(-1)?;
    Ok(())
}).unwrap();
assert_eq!(counter.get_value(), 12);

// A failed transaction leaves the value exactly as it was, including at the wrapping extreme.
let failing = counter.transaction("overflow-then-fail", |c| {
    c.increase_by(i64::MIN)?; // the self-inverse extreme
    Err("abort".into())
});
assert!(failing.is_err());
assert_eq!(counter.get_value(), 12);
```

Two clients converge by exchanging increments through a connectivity backend rather than
sharing memory directly — see the two-client walkthrough in
[`docs/getting-started.md`](getting-started.md#syncing-two-clients), which uses a counter for
exactly this.

## Rationale

**Why an operation-based delta instead of a stored value with a merge function.** Two replicas
each adding `+1` concurrently must both survive as `+2` total, not collapse to one `+1` the way
a last-writer-wins value would. Recording the delta as the operation, rather than the resulting
value, is what makes concurrent increments commute regardless of arrival order.

**Why wrapping instead of checked or saturating arithmetic.** A CRDT operation must have the
same effect no matter which replica applies it or in what order, or replicas can diverge.
Checked arithmetic can error on one replica's starting value and succeed on another's for the
identical operation; saturating arithmetic loses information about how far past the boundary
an increment actually went, and once two replicas saturate at the same clamp their subsequent
increments stop being distinguishable from each other. Wrapping addition has neither problem —
it always succeeds and it is always invertible — which is also why rollback can use a simple
recorded inverse instead of the exact-restore snapshot [`Variable`](variable.md) needs for its
own, non-arithmetic value.

## Code Map

| Concern | Location |
|---------|----------|
| Public API: `increase`, `increase_by`, `get_value`, `transaction` | `src/datatypes/counter.rs` |
| The CRDT state, wrapping arithmetic, and its inverse | `src/datatypes/crdts/counter_crdt.rs` |
| The wire operation body | `src/operations/body/counter.rs` |
| Snapshot encoding and decoding | `src/datatypes/crdts/counter_crdt.rs` (`SnapshotCodec` impl) |

| Verified by | Tests |
|-------------|-------|
| Cumulative increments, including a negative delta | `can_new_and_increase_counter` in `src/datatypes/crdts/counter_crdt.rs` |
| Wrapping at the `i64` boundary in both directions | `can_wrap_counter_arithmetic_at_i64_boundaries` in `src/datatypes/crdts/counter_crdt.rs` |
| Rollback is exact, including the `i64::MIN` self-inverse extreme | `can_return_and_apply_a_counter_rollback_action` in `src/datatypes/crdts/counter_crdt.rs` |
| Snapshot round-trips and rejects a malformed length | `can_serialize_and_deserialize_counter_crdt`, `can_reject_an_invalid_counter_snapshot_length` in `src/datatypes/crdts/counter_crdt.rs` |
| A transaction of several increments commits or undoes as a unit through the public API | `can_use_transaction` in `src/datatypes/counter.rs` |
| Two clients converge over a real exchange | `LocalConnectivity` doctest in `src/connectivity/local_connectivity.rs` |

## Related Concepts

- [`docs/architecture.md`](architecture.md) — the shared layer stack and the convergence limits every datatype, including this one, is bound by
- [`docs/transaction-and-rollback.md`](transaction-and-rollback.md) — the transaction scope and rollback-by-inverse mechanism this document assumes
- [`docs/variable.md`](variable.md) — a sibling datatype whose rollback shape (exact restore) contrasts with a counter's (recorded inverse)
- [`docs/getting-started.md`](getting-started.md) — a runnable first Counter and a two-client sync walkthrough
