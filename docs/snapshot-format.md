# Snapshot Format

Snapshots encode a complete CRDT state for replacement on a receiving replica. All three
datatypes use the same revision-one envelope; their uncompressed payloads differ as described
below. This document owns the binary layout. [Connectivity](connectivity.md#the-bundled-backends)
describes snapshot ownership and delivery, and [Error Handling](error-handling.md) describes
encoding and decoding failures.

## Encoding layers

```text
CRDT state → datatype payload → optional Zstd → QTSN header + stored payload
```

`SnapshotWriteOptions` selects Bitcode or Postcard and compression independently. Cargo
features determine available capabilities; when both codecs are enabled, the writer can
select either. Readers select the decoder from the header, independently of their own writer
policy. Unknown or disabled capabilities are rejected without fallback.

The fixed envelope and Map metadata below specify exact byte widths. Tables of codec fields
describe logical schemas in declaration order, not fixed offsets or native Rust memory layouts.
Bitcode (`bitcode` 0.6) and Postcard (`postcard` 1.x with Serde derives) determine their actual
field encoding. In particular, an `i64` field does not imply an eight-byte payload, and codec
integers do not use the fixed little-endian layout of the envelope.

## Common envelope

The header is exactly 24 bytes. All multibyte integers in this header are unsigned and
little-endian. Offsets are relative to the beginning of the snapshot.

| Offset | Bytes | Field | Value or meaning |
|--------|-------|-------|------------------|
| 0 | 4 | Magic | ASCII `QTSN` |
| 4 | 1 | Revision | `1` |
| 5 | 1 | Datatype | `1` Counter, `2` Variable, `3` Map |
| 6 | 1 | Codec | `1` Bitcode, `2` Postcard |
| 7 | 1 | Compression | `0` none, `1` Zstd |
| 8 | 8 | Stored length | Bytes after the header |
| 16 | 8 | Raw length | Payload bytes before compression |
| 24 | Stored length | Stored payload | Raw payload or one Zstd frame |

Without compression, stored and raw lengths must match. With compression, the whole raw
payload is compressed once, including Map counts, CUID table, and block lengths. There is no
per-block compression.

The writer emits one ordinary Zstd frame at level 1 with checksum and content size. The reader
accepts one ordinary frame with or without those optional fields, verifies a checksum when
present, and requires exactly the declared raw output size. It rejects concatenated frames,
skippable frames, trailing bytes, and dictionary IDs; it supplies no external dictionary.
Both sides use a maximum reference window of 8 MiB.

## Shared value conventions

A CUID is the 16 ASCII bytes of the Qortoo identifier, using letters, digits, `-`, or `_`.
It is not a UUID or a binary integer. The nil CUID is ASCII `0000000000000000`, not sixteen
zero bytes. Lamport clocks are logical `u64` values.

JSON values are stored as UTF-8 text containing exactly one valid JSON value. Validation
preserves the original text, including whitespace and number spelling; it does not parse and
reserialize a JSON object. A live JSON `null` is distinct from an absent Map value.

## Counter payload

Kind `1` encodes one `SnapshotCounter` using the selected codec.

| Field | Logical type | Meaning |
|-------|--------------|---------|
| `value` | `i64` | Current cumulative counter value |

Every `i64` value is valid. No timestamp or per-client contribution table is included.
The codec must consume the complete raw payload.

## Variable payload

Kind `2` encodes one `SnapshotVariable` using the selected codec.

| Field | Logical type | Meaning |
|-------|--------------|---------|
| `lamport` | `u64` | Winning timestamp's Lamport clock |
| `cuid` | `[u8; 16]` | Winning timestamp's CUID |
| `value` | String | Original JSON text |

The timestamp preserves LWW ordering after snapshot installation. A normal state requires
`lamport > 0`; a nil CUID is allowed. The sole zero-Lamport state is the initial state:
nil CUID and exactly the four bytes `null`. Whitespace around `null` does not satisfy that
initial-state exception. Encoding and decoding both validate this invariant and the JSON.
The codec must consume the complete raw payload.

## Map payload

Kind `3` uses fixed metadata followed by independently codec-encoded blocks. Entry counts
include both live entries and tombstones. Map currently has an internal CRDT implementation;
its public handle and builder are pending.

```text
entry_count: u64 little-endian
cuid_count:  u32 little-endian
cuids:       cuid_count × 16 raw ASCII bytes
repeat ceil(entry_count / 32) times:
    block_length: u64 little-endian
    block:        block_length bytes encoded with the selected codec
```

An empty Map has `entry_count = 0`, `cuid_count = 0`, and no blocks: its raw payload is
12 bytes. It still has the common envelope and may be compressed.

### CUID table

The writer visits entries in ascending key order and adds each distinct timestamp CUID at
its first occurrence. Rows refer to this snapshot-local table by zero-based index. The table
is not sorted by CUID or frequency. Both live entries and tombstones contribute CUIDs.

The reader requires unique, valid CUIDs and `cuid_count <= entry_count`; every used row must
reference an existing table entry. It does not require the writer's first-occurrence order
or require every table entry to be referenced.

### Blocks and rows

Every block encodes a fixed array `[SnapshotMapRow; 32]`, including the final partial block.
Each row has this logical schema:

| Field | Logical type | Meaning |
|-------|--------------|---------|
| `key` | String | UTF-8 Map key |
| `lamport` | `u64` | Entry timestamp's Lamport clock |
| `cuid_index` | `u32` | Index into the CUID table |
| `value` | `Option<String>` | `Some(JSON text)` for live entries; `None` for tombstones |

For example, `Some("null")` is a live JSON null; `None` is a deletion tombstone. No separate
live/deleted byte is defined by this format: the selected codec encodes the option.

Rows are strictly ascending by Rust string key order across all blocks. Duplicate or unsorted
keys are invalid. Each used row requires a positive Lamport clock, a valid CUID index, and
valid JSON when live. The snapshot decoder imposes no additional key naming rule.

Unused rows in the final block must exactly equal this padding row:

```text
key = "", lamport = 0, cuid_index = 0, value = None
```

Padding is not an entry and its CUID index is not resolved. Thus one entry uses one block
with 31 padding rows; 32 entries use one full block; 33 entries use two blocks with 31 padding
rows in the second. The entry count determines the number of blocks and used rows; there is
no separate block count or used-row count. Each codec must consume exactly its block's bytes,
and no bytes may follow the last block.

Decoding rebuilds the live and tombstone counts and sets each entry's local
`approved_sseq_upper` to `None`. GC approval is not serialized, and snapshot installation
alone does not authorize tombstone reclamation.

## Receiver validation and state replacement

`SnapshotDecodeLimits` bounds complete input bytes (including the header), raw payload bytes,
and Map entries (including tombstones). Limits are inclusive and independent of writer policy;
they are not a peak-memory bound. See
[Snapshot Receiver Policy](client-and-datatype-builder.md#snapshot-receiver-policy) for configuration.

Readers reject truncated fields, inconsistent lengths, malformed codec data, invalid domain
values, and excess input. Unknown revisions or kinds are unsupported; a known kind that
differs from the target CRDT is invalid. Decoding builds and validates a candidate before
`Crdt::deserialize` replaces the existing state. A failed decode leaves that state intact.

## Implementation and format checks

| Concern | Source |
|---------|--------|
| Header, fixed lengths, codec dispatch, complete consumption | `src/datatypes/crdts/snapshot/wire.rs` |
| Zstd framing and bounded output | `src/datatypes/crdts/snapshot/compression.rs` |
| Counter / Variable / Map schemas | `src/datatypes/crdts/snapshot/{counter,variable,map}.rs` |
| Capability IDs and receiver limits | `src/types/snapshot.rs` |
| Candidate installation and operation-specific errors | `src/datatypes/crdts/crdt.rs` |
| Golden Map bytes for 0, 1, and 33 entries, both codecs without compression | `src/datatypes/crdts/snapshot/tests/fixtures/` |
| Golden comparison, malformed input, domain validation, and JSON preservation | `src/datatypes/crdts/snapshot/tests/mod.rs` |
