# Error Handling

Qortoo separates *what went wrong* from *how the SDK responds*. An error is a typed value
naming the failure; a recovery action is the single policy that applies to it. Some errors
are answered by the caller and never leave the call that produced them; others are decided
by the SDK, which changes the datatype's lifecycle and its synchronization schedule and then
tells the application what happened. This document defines which errors exist, which of the
two destinations each one has, and what the SDK does on the way.

## Model

| Term | Meaning |
|------|---------|
| Typed error | A value naming a failure, carrying a numeric code and a human-readable message |
| Caller-facing error | An error returned from the call that produced it; the caller decides what to do |
| Routed error | An error the SDK acts on, pairing it with a recovery action before any consumer sees it |
| Recovery action | One whole policy: the lifecycle effect and the scheduling effect that must happen together |
| Translation boundary | The single place that pairs an error with its action, kept next to the error definition |
| Wire error | A failure reported by the responder inside the exchange package, translated on arrival |
| Internal reason | A crate-private cause behind an internal error, used to pick a routing before the cause is erased |

An error reaches one of two destinations, never both. A caller-facing error is a return value:
naming a key this client does not manage, writing to a datatype whose state forbids it, or
handing a datatype a value it cannot represent. Nothing about the datatype changes and the SDK
takes no action. A routed error is one the caller could not have prevented and cannot fix in
place — a backend that timed out, a server that rejected the subscription — so the SDK pairs it
with a recovery action, applies that action, and reports the error through the datatype's
handler.

Routing a caller-facing error is a defect, not a fallback: the translation boundary refuses
those variants rather than assigning them a default action.

```mermaid
flowchart TD
    WIRE["wire error<br/>reported by the responder"]
    CONN["backend error"]
    INT["internal reason"]
    DE["typed datatype error"]
    MAP["translation boundary"]
    PAIR["error + recovery action"]
    LOOP["sync path:<br/>event loop applies the action"]
    COMMIT["commit path:<br/>the committing thread applies the action"]
    HANDLER["error handler"]
    CALLER["the calling code"]
    DIRECT["caller-facing error"]

    WIRE --> DE
    CONN --> DE
    INT --> DE
    INT -.->|routing chosen before the cause is erased| PAIR
    DE --> MAP --> PAIR
    PAIR --> LOOP --> HANDLER
    PAIR --> COMMIT --> HANDLER
    DIRECT --> CALLER
```

## Error Reference

### Client errors (codes 100–)

Returned directly to the caller; never routed.

| Variant | Code | Trigger |
|---------|------|---------|
| `InvalidCollectionName` | 100 | The collection name fails naming validation |
| `FailedToSubscribeOrCreateDatatype` | 101 | The datatype key is invalid, or the client already holds that key |

### Datatype errors (codes 200–)

The public error type for datatype operations. Caller-facing variants are returned from the
call; routed variants are paired with an action.

**Caller-facing.**

| Variant | Code | Meaning |
|---------|------|---------|
| `TransactionFailed` | 201 | The transaction closure returned an error, or the commit could not complete |
| `Disallowed` | 205 | Access denied for a reason other than state or readonly configuration, such as a key this client does not manage |
| `NotWritable` | 206 | The datatype's lifecycle state does not permit writes |
| `ValueConversion` | 214 | A value could not be converted to or from the datatype's representation; nothing was applied |

**Routed.**

| Variant | Code | Meaning | Recovery action |
|---------|------|---------|-----------------|
| `Internal` | 202 | An SDK fault, not actionable by the application | `Disable` \* |
| `ReadonlyViolation` | 207 | The server reports a write from a client configured as readonly | `Disable` |
| `SyncFailed` | 210 | A transient exchange failure, such as a timeout or a server-side internal error | `RetryWithBackOff` |
| `PushBufferExceededMaxMemSize` | 211 | The committed transaction does not fit in the push buffer | `RollbackTransaction` |
| `ServerRejected` | 213 | The server refused the operation; the reason is carried inside | `Resubscribe` for `MissingSubscription`; otherwise `Disable` |

\* except the non-sequential-sequence reason, which chooses `RollbackTransaction` at its
creation site — see **Creation-site routing** below.

`ReadonlyViolation` reaches both destinations depending on who detects it. A local write by a
readonly client is refused as `NotWritable` before anything is applied; the same violation
reported by the server arrives as a routed error and disables the datatype.

### Server rejection reasons

Carried inside a `ServerRejected` error. Lifecycle operations added later extend this set
without changing the datatype error type. `ServerRejectReason::Display` renders a lowercase
reason phrase and preserves its detail verbatim, using the shared `sync` catalog wording.
`ServerRejected` includes that display text under `E213`, for example
`[E213] server rejected: creation failed: datatype already exists`. Reasons do not introduce
another code; `Debug` remains available for inspecting the Rust variant. New reasons must
implement the exhaustive display match and update the complete-output and recovery assertions
in `can_describe_server_rejection_reasons`.

| Variant | Trigger |
|---------|---------|
| `CreateFailed` | The server refused to create the datatype, for example because it already exists |
| `ResourceNotFound` | The requested resource does not exist or has an incompatible type |
| `MissingSubscription` | The server has no subscription entry, for example after losing state |
| `ProtocolViolation` | The push violated the wire protocol, such as an unexpected state transition or a type mismatch |

### Backend errors

Crate-internal and not part of the public surface; they stay internal until custom backends
become a public extension point.

| Variant | Trigger | Becomes |
|---------|---------|---------|
| `TimedOut` | The backend did not respond in time | `SyncFailed` |

### Wire errors (codes 300–)

Set by the responder in the exchange package and translated on arrival. Names mirror the
server rejection reasons where a counterpart exists.

| Variant | Code | Becomes |
|---------|------|---------|
| `ProtocolViolation` | 301 | `ServerRejected(ProtocolViolation)` |
| `ReadonlyViolation` | 302 | `ReadonlyViolation` |
| `CreateFailed` | 303 | `ServerRejected(CreateFailed)` |
| `ResourceNotFound` | 304 | `ServerRejected(ResourceNotFound)` |
| `MissingSubscription` | 305 | `ServerRejected(MissingSubscription)` |
| `ServerInternalError` | 306 | `SyncFailed`, treated as transient |

### Recovery actions

| Action | Lifecycle effect | Scheduling effect | Produced by |
|--------|------------------|-------------------|-------------|
| `NotifyOnly` | none; the handler is told and nothing changes | normal | *reserved* |
| `RetryWithBackOff` | none | exponential backoff | `SyncFailed` |
| `RollbackTransaction` | undo the pending transaction | never reaches the event loop | `PushBufferExceededMaxMemSize`, non-sequential sequence |
| `Resubscribe` | discard pending transactions, reset checkpoint, request a snapshot | one immediate attempt, including manual mode | `ServerRejected(MissingSubscription)` |
| `ResubscribeWithBackOff` | reset local state, return to subscribing | exponential backoff | *reserved* |
| `Disable` | disable the datatype | stopped; further sync requests are refused | `Internal`, other `ServerRejected` reasons, `ReadonlyViolation`, repeated missing subscription during immediate recovery |

## Rules and Guarantees

**One source for message wording.** `src/errors/messages.rs` owns product-authored return-error
text, including validation details, snapshot diagnostics, observability setup, and C ABI
argument errors. Error types, numeric codes, and recovery actions stay in their existing
modules. The C ABI includes the same source file through a private module; it does not expose
the catalog as public Rust API. External causes, application-provided errors, logs, raw panic
messages, and independent test expectations remain at their call sites. Add new return-error
wording to the catalog as a constant or formatting function.

### Message catalog rules

Keep the catalog in one file, with these crate-private inline modules. Classify by the
failure being described, even when another layer constructs or returns the error.

| Module | Owns |
|--------|------|
| `client` | Collection/key validation and datatype registration conflicts |
| `datatype` | Local write admission, lifecycle restrictions, transactions, and CRDT execution |
| `sync` | Connectivity, event-loop channels, push/pull, checkpoints, subscriptions, and server rejection |
| `snapshot` | Snapshot structure, versions, payload validation, and UID/timestamp decoding |
| `value` | User-facing JSON value encoding, decoding, and conversion |
| `observability` | Telemetry configuration, initialization, exporters, and shutdown |
| `ffi` | C argument/handle validation, foreign callback failures, and boundary error translation |

For example, a local state restriction on `sync()` belongs to `datatype`, while an invalid
exchange checkpoint belongs to `sync`. Invalid JSON inside a snapshot belongs to `snapshot`;
invalid JSON passed to the value API belongs to `value`. C-specific configuration encoding
(such as an unknown integer log format) belongs to `ffi`; telemetry setup failures belong to
`observability` regardless of whether Rust or C called the API.

- Use `SCREAMING_SNAKE_CASE` constants for fixed text and `snake_case` functions for text
  containing runtime values. Name the failure or diagnostic purpose, without `ERR_`, `MSG_`,
  or a redundant module prefix.
- Import the catalog domain with a short alias in each scope. When using one domain, use
  `use crate::errors::messages::snapshot as msg;` and call
  `msg::unsupported_version(version)`. When using multiple domains, use `<domain>_msg`
  aliases, such as `datatype_msg` and `snapshot_msg`, to keep ownership visible. FFI callers
  use the same aliases under `crate::messages`. Do not use glob imports.
- Use `_PREFIX` for text that precedes an error payload, `_TEMPLATE` for an existing
  runtime-format template, and `_CONTEXT` or `_FIELD` for diagnostic labels. Keep fixed text
  before formatting functions within each module. Prefer a formatting function for new
  messages with runtime values; do not add new runtime-format templates.
- Keep authored diagnostic prose and punctuation in the catalog. Core error types render
  their `[E<number>]` identifier from `code()`; C-only boundary labels stay in the catalog. Pass values and external
  causes into formatters; do not assemble new error prose at call sites. Reuse a formatter
  when the meaning and output are the same.
- Keep error variants, numeric codes, and `RecoveryAction` decisions in their current
  definition/translation modules. Catalog modules are editorial ownership, not new error
  types or code ranges. Callers must not parse message text to decide behavior.
- When adding an error, select its domain, reuse or add its text there, and verify the
  complete output at the existing error/API test boundary. Expected strings must be
  independent literals, not assembled from catalog constants. Keep C tests specific to
  boundary behavior; core semantics belong in core tests.
- Structural moves and renames must preserve emitted text, codes, public APIs, and recovery
  behavior. Treat wording edits as deliberate behavior changes, review the final text, and
  update the affected output assertions. New wording should state the failed operation or
  restriction, the specific reason, and useful context such as a key or expected/actual value;
  preserve an external cause when available. Do not infer retry advice from the wording.

Use a lowercase diagnostic phrase after the diagnostic identifier, with `: ` separating a
summary from its details. Preserve proper names and acronyms such as JSON, UTF-8, and UID.
State the actual validation unit (`bytes` for collection names and datatype keys), and say
`ASCII` when only ASCII letters or digits are accepted. Describe state restrictions as
`cannot <operation> ... in <State> state`, and type conflicts with the requested and existing
types. A caught Rust panic at the C boundary must be described as caught, rather than as a
panic that crossed the boundary.

### Diagnostic identifiers

Format coded errors as `[E<number>] summary: detail`, for example
`[E100] invalid collection name: ...`. `E` identifies an error; `W` is reserved for
warnings. No warning API or warning codes are currently defined. Retryable errors still use
`E`; retry policy and the severity of a log event are independent of this identifier.

Use the existing numeric code without renumbering or filling gaps. `ClientError`,
`DatatypeError`, `PushPullError`, and `ObservabilityError` expose `pub const fn code(&self) -> i32`.
Their exhaustive matches own the Rust-to-binding mapping, and core message identifiers use
that same method. FFI calls `code()` directly, including handler callbacks; it has no separate
variant mapping or fallback for a newly added core error. Existing `repr(i32)` discriminants
remain unchanged and must agree with `code()`.

When adding a coded error, update its discriminant (where present), exhaustive `code()` match,
catalog prose, and the independent cases in `can_preserve_unique_codes_and_matching_identifiers`
and complete-output tests together. The code test pins each numeric value, checks matching
identifiers, and rejects collisions across the listed variants. Existing CI runs these tests
with all features. C observability constants are checked against their core counterparts.
At the C boundary, verify that the message identifier agrees with `QortooError.code`.
Bindings must use the numeric field, never parse the message.

Internal causes retain their descriptive text under the final error identifier. For example,
a connectivity timeout becomes `[E210] sync failed: connection timed out: ...`; it does not
need a second identifier. Boundary-only invalid arguments use `E998`, contained panics use
`E999`, and observability setup errors use their existing codes `900` through `906`.

### Error routing rules

**One destination per error.** Every error is either returned to its caller or routed, and the
classification is a property of the variant, not of the situation. The translation boundary
treats a caller-facing variant as a defect and refuses it rather than choosing a default.

**One action per routed error.** The translation from error to action lives next to the error
definition, so a new routed variant is added in one place and every consumer matches
exhaustively. No consumer decides policy for itself.

**Recovery actions are whole policies.** An action names a lifecycle effect and a scheduling
effect together. There is no way to express a combination the SDK does not intend, such as
disabling a datatype while leaving its synchronization scheduled.

**One dispatch point for lifecycle effects.** Both consumer paths apply the lifecycle half of
an action through the same code, so a rollback or a disable means the same thing regardless of
which path decided it. Recovery and the error-handler snapshot share one mutable-state
boundary. Detach and handler task scheduling execute after that lock is released, with
detach before the Disabled notification and state-change dispatch before error dispatch.
Dispatch order does not guarantee callback completion order across notifications.

**The sync path.** A failed exchange is routed, the event loop takes its scheduling mode from
the action, the lifecycle effect is applied, and the error is reported to the datatype's
handler. Retrying uses exponential backoff between 500 ms and 30 s, and is not capped by an
attempt count; a disabled datatype refuses further sync requests. See
[`docs/event-loop.md`](event-loop.md).

**The commit path.** Undoing a transaction never travels through the event loop, because the
thread that committed is still the one that has to undo it. When enqueuing a committed
transaction fails, the pending transaction is restored so its undo actions still match, the
undo is applied before the owning standalone write or explicit transaction returns the enqueue
error. The handler receives one additional notification.

**Creation-site routing.** An internal error erases its cause into a message. A cause that
needs a routing other than the internal default must therefore choose its action before the
erasure, at the point the error is created. Every other cause defers to the single table.

**Missing-subscription recovery discards pending transactions.** `Resubscribe` resets the
checkpoint and fetches a snapshot on the same instance without an intermediate `Disabled`
transition or manager detach. The current local CRDT remains observable until the snapshot
replaces it; its value is not a server acknowledgement. Writes committed while that snapshot
is in flight are also discarded. Recovery errors and lifecycle changes use the existing handlers.
A successful recovery completes the original `sync()` successfully. Recovery also clears
the client GC acknowledgement before requesting the new snapshot. A repeated missing
subscription during the immediate recovery attempt disables the datatype instead of looping.
`ResubscribeWithBackOff` remains reserved. History-pruned protocol errors still use
`Disable` and do not trigger snapshot recovery.

**Limits on the guarantees.** Results of operations inside a transaction callback are
tentative until the enclosing transaction returns success. A standalone write or explicit
transaction returning `Ok` confirms local commit, not server acknowledgement. Stack traces attached to internal errors are
diagnostic only; a platform that cannot produce one omits the trace and preserves the typed
error, and trace formatting never replaces an application error or escapes as a panic across a
language boundary.

## Behavior

**A backend that times out.** The exchange fails, the error becomes a transient sync failure,
and the action is to retry with backoff. Nothing about the datatype changes: its value, its
lifecycle state, and its buffered transactions are all intact, and the handler is told. The
next attempt happens after the backoff interval, which grows with consecutive failures.

**A server that rejects the subscription.** The wire error becomes a server rejection and the
action is to disable. The lifecycle effect runs first, so the datatype is already disabled when
the error notification arrives — a handler that inspects the datatype it is given observes the
disabled state, not the state the request was sent in. The event loop stops and refuses further
sync requests.

**A push buffer that is full.** The transaction closure returned success, but the transaction
does not fit. The action is to undo, which happens on the committing thread rather than in the
event loop. The datatype returns to its pre-transaction value and the handler is told. The call
that made the write still returned its value.

**A value the datatype cannot represent.** Converting the value fails before any operation is
created, so nothing is applied, no transaction opens, and the failure is returned from the call
that supplied the value. Nothing is routed and no handler is involved.

**A transaction that arrives out of sequence.** The push buffer rejects a transaction whose
sequence number is not contiguous. This is an internal fault, whose default routing would
disable the datatype, but it is recoverable by undoing the transaction — so the routing is
chosen where the error is created, and the transaction is undone instead.

## Rationale

**A recovery action is one policy, not two independent axes.** Scheduling and lifecycle
effects are not independent: disabling a datatype while leaving its synchronization scheduled
is not a state the system should be able to express. Naming the valid policies directly makes
the invalid combinations unrepresentable and lets every consumer match exhaustively instead of
reasoning about which pairs are coherent.

**The error-to-action table lives next to the errors.** A central match in the event loop would
make the loop the place every new error variant is remembered, and would give the loop
authority over policy for errors it never sees — the commit path among them. Keeping the
translation next to the error definition means adding a variant is a local change.

**Caller-facing variants have no default action.** Assigning one would make a misrouted error
silently produce a lifecycle change, which is worse than the error itself. Refusing them turns
a routing mistake into a failure that is found immediately rather than a datatype that quietly
disables itself.

**Internal causes are erased, so their routing is chosen first.** The public surface should not
grow a variant for every internal fault, but a few of those faults are recoverable in a way the
generic internal error is not. Choosing the action at the creation site keeps the public error
type small without losing the distinction that matters.

**Wire error names mirror the rejection reasons they become.** The translation between the two
is then obvious to read and hard to get wrong, and a new server-side reason has an evident
counterpart.

**Errors compare by variant, not by message.** Tests and callers ask whether two failures are
the same kind, which is the question that has a stable answer; message strings carry context
that is expected to differ between occurrences of the same failure.

**Public error types are extensible.** Every public error enum is marked non-exhaustive and
carries explicit numeric codes, so adding a variant does not break downstream code and codes
stay stable for language bindings that transmit them.

## Code Map

| Concern | Location |
|---------|----------|
| Datatype errors, rejection reasons, internal causes, recovery actions, and the translation boundary | `src/errors/datatypes.rs` |
| Client errors | `src/errors/clients.rs` |
| Shared Rust and C ABI return-error wording | `src/errors/messages.rs` |
| Backend errors | `src/errors/connectivity.rs` |
| Wire errors and their translation | `src/errors/push_pull.rs` |
| The boxed error alias and the logging macro that captures a filtered stack trace | `src/errors/mod.rs` |
| Observability-specific errors | `src/errors/observability.rs` |
| Applying the lifecycle half of an action | `src/datatypes/mutable.rs` (`apply_action`) |
| The sync-path consumer | `src/datatypes/wired.rs` (`handle_error`) |
| Deriving the scheduling mode from an action | `src/datatypes/event_loop.rs` |
| The commit-path consumer | `src/datatypes/transactional.rs` (`end_transaction`) |
| Sequence and capacity checks that produce routed errors | `src/datatypes/push_buffer.rs` |

| Verified by | Tests |
|-------------|-------|
| Errors compare by variant rather than by message | `can_compare_errors` in `src/errors/mod.rs` |
| Complete error text retains its format and detail | `can_preserve_complete_error_messages` and `can_preserve_observability_error_messages` in `src/errors/mod.rs` |
| The logging macro preserves the typed error | `can_use_err_macro` in `src/errors/mod.rs` |
| An unavailable stack trace is omitted rather than fatal | `can_omit_an_unavailable_stack_trace` in `src/errors/mod.rs` |
| Invalid collection errors describe naming requirements | `can_describe_collection_name_requirements` in `src/errors/clients.rs` |
| Every observability error variant is distinct | `can_describe_every_variant_distinctly` in `src/errors/observability.rs` |
| An invalid collection name is refused at the caller | `can_reject_invalid_collection_names` in `src/clients/client.rs` |
| A create failure disables the datatype and detaches it | `can_auto_detach_after_create_failure_disables_datatype` in `src/clients/client.rs` |
| A subscribe failure disables the datatype and detaches it | `can_auto_detach_after_subscribe_failure_disables_datatype` in `src/clients/client.rs` |
| An unmanaged key is refused at the caller | `can_reject_unsubscribe_for_unmanaged_key` in `src/clients/client.rs` |
| A commit that cannot be enqueued is undone | `can_rollback_on_enqueue_failure` in `src/datatypes/transactional.rs` |
| The server refuses protocol violations and pushes from disabled or unsubscribed clients | `can_reject_subscribed_push_from_unsubscribed_client`, `can_reject_type_mismatch_in_subscribed_push`, `can_reject_push_from_disabled_client` in `src/connectivity/local_datatype_server.rs` |
| An error notification carries the datatype and its state | `can_notify_error` in `src/datatypes/handler.rs` |

## Related Concepts

- [`docs/architecture.md`](architecture.md) — where the two consumer paths sit in the layer model
- [`docs/event-loop.md`](event-loop.md) — backoff, the stopped mode, and how scheduling responds to an action
- [`docs/transaction-and-rollback.md`](transaction-and-rollback.md) — what undoing a transaction does
- [`docs/handler-system.md`](handler-system.md) — how an error reaches the application
- [`docs/datatype-state.md`](datatype-state.md) — the lifecycle states an action moves a datatype between
- [`docs/connectivity.md`](connectivity.md) — where backend and wire errors originate
- [`docs/variable.md`](variable.md) — the value contract whose violations surface as conversion errors
- [`docs/go-binding.md`](go-binding.md) — how numeric codes cross a language boundary
