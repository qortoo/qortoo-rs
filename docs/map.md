# Map

A Map stores one independently versioned JSON value per key. Use `build_map()` on a
[datatype builder](client-and-datatype-builder.md) to create or subscribe to a Map.
GC approval and physical tombstone reclamation are not enabled.

```rust
let map = client.subscribe_or_create_datatype("settings").build_map()?;
map.put("theme", "dark")?;
assert_eq!(map.get::<String>("theme")?.as_deref(), Some("dark"));
```

`put` and `remove` return the previous value as an `Option<serde_json::Value>`.
`put_raw`, `get_raw`, and `remove_raw` preserve the original JSON bytes.
`len`, `is_empty`, `contains_key`, and `to_json` report only live entries.
`to_json` captures a consistent root under the read lock and decodes it after releasing the
lock. Use `transaction` and its callback handle to commit multiple writes together.
Datatype handlers receive a `DatatypeSet::Map` variant for Map events. Include this variant
when matching exhaustively on `DatatypeSet`.

## Model

| Term | Meaning |
|------|---------|
| Key | Any UTF-8 string, including the empty string; whitespace, embedded NUL, and Unicode are preserved |
| Live entry | A timestamp paired with immutable JSON bytes |
| Tombstone | A timestamp with no live value, preventing stale writes from resurrecting a key |
| Absent entry | A key with neither a live value nor a tombstone |
| Winning entry | The entry with the greatest timestamp for one key |

Entries use a persistent `rpds::RedBlackTreeMapSync`: a batch fork shares its root in O(1),
and writes copy only the search path rather than the entire collection. Keys and payloads
are immutable `Arc` values. Counts and approval fields belong to each fork's state.

## Rules and Guarantees

- Values follow the [Variable JSON contract](variable.md): exactly one UTF-8 JSON value,
  preserved byte-for-byte. JSON `null` is a live value, distinct from deletion and absence.
- A newer timestamp wins per key. An older operation is a successful no-op. Equal timestamps
  are accepted only for identical raw bytes and entry state; conflicting payloads are errors.
- The public Map write API validates Put values. The in-process backend trusts
  those values. A network backend must validate Put values from clients before
  applying them. Clients trust Put values received during synchronization.
  Snapshot decoding validates JSON.
- Local Put always records a write. Local Remove on an absent or tombstoned key records
  nothing, consumes no operation ID, and retains any existing tombstone. Remote Remove
  creates or advances a tombstone even for a missing key.
- An accepted remote operation advances the observed Lamport clock, including stale and
  duplicate operations. An invalid operation does not advance it.
- Rollback restores the previous entry, timestamp, approval field, live count, and tombstone
  count/byte total. Actions run in reverse transaction order. Tombstone accounting uses
  `key UTF-8 bytes + 42` logical bytes per entry; it does not measure allocator usage.
- Debug and display output expose sizes and counts rather than map keys or JSON payloads.

The transactional layer provides a closure boundary under its normal transaction gate and
mutable write lock. This lets typed writes decode the previous value before mutation: raw
JSON can be valid while exceeding `serde_json::Value`'s number range or nesting limit.
A failed conversion leaves earlier successful operations in the transaction intact unless
the caller propagates the error and aborts the transaction.

## Behavior

Putting `null` at `k` creates a live entry and returns the previously stored bytes, if any.
Removing `k` replaces it with a tombstone and returns `null`'s bytes. Removing it again is a
local no-op. A delayed Put older than the tombstone cannot recreate `k`; a newer Put can.

### Snapshot format

The [Map snapshot payload](snapshot-format.md#map-payload) uses the common QTSN envelope,
a snapshot-local CUID table, and codec-encoded blocks of 32 rows in strictly ascending key
order. Both live entries and tombstones retain their timestamps; live JSON null remains
distinct from a tombstone. The format document specifies fields, padding, and validation.
Decoding builds a validated temporary map before replacing state. The snapshot receiver
counts live entries and tombstones toward `SnapshotDecodeLimits::max_map_entries`
(1,000,000 by default). An operation stream can grow a Map beyond this limit, at which point
a receiver with that limit will reject its snapshot.
Counts are recomputed and local GC approvals are cleared. Validated snapshot approval and
GC integration remain future work; snapshots alone do not authorize reclamation.

## Rationale

Per-key timestamps let unrelated writes commute. Keeping tombstones preserves deletion
ordering. Persistent entries keep candidate batches isolated without copying the whole map,
while retaining exact prior entries makes rollback independent of the operation's inverse.

## Code Map

| Responsibility | Source |
|----------------|--------|
| Public API | `src/datatypes/map.rs` |
| Put/Remove bodies, constructors, and memory sizes | `src/operations/body/map.rs` |
| Shared timestamped node (`TimedNode`), payload, and approval metadata | `src/datatypes/crdts/timed_node.rs` |
| Entry transitions, accounting, fork, and rollback | `src/datatypes/crdts/map_crdt.rs` |
| Snapshot codec | `src/datatypes/crdts/snapshot/map.rs` |
| Permutations, rollback, fork isolation, unsupported operations, malformed snapshots | `src/datatypes/crdts/tests/map_crdt.rs` |
| No-op recording and remote clock validation | `src/datatypes/mutable.rs` |
| Typed predecode boundary, commit failure, and concurrent previous-value tests | `src/datatypes/transactional.rs` |

## Related Concepts

- [Core Types](core-types.md) — timestamp ordering and identity
- [Transaction and Rollback](transaction-and-rollback.md) — operation recording and commit
- [Variable](variable.md) — shared JSON representation
- [Error Handling](error-handling.md) — caller errors and routed remote failures
