use thiserror::Error;

/// Internal SDK error reason, used by the event loop for action routing.
///
/// Not exposed to users. Internal callers use this to construct `DatatypeError::Internal`
/// via `into_error()`.
#[derive(Debug, Error)]
pub(crate) enum InternalReason {
    /// A server snapshot could not be deserialized into the CRDT.
    ///
    /// Route: `apply_snapshot_transaction()` → `pull_handler::apply_subscribe_response()`
    /// → `mapping()` → `RecoveryAction::Disable`.
    #[error("deserialize: {0}")]
    Deserialize(String),
    /// A CRDT operation failed to execute.
    ///
    /// Routes:
    /// - remote: `execute_transactions()` → `mapping()` → `RecoveryAction::Disable`
    /// - local: returned directly to the user API caller (does not reach the event loop)
    #[error("execute operation: {0}")]
    ExecuteOperation(String),
    /// An event-loop channel operation failed.
    ///
    /// Routes:
    /// - `receive_event()` Err → `handle_error(_, RecoveryAction::Disable)` directly,
    ///   bypassing `mapping()` (the loop is stopping anyway)
    /// - send/response failures and stopped-loop rejections: returned directly to the
    ///   API caller (e.g., `sync()`), without event-loop routing
    #[error("event loop: {0}")]
    EventLoop(String),
    /// Cseq was non-sequential in the push buffer.
    ///
    /// Routed at the enqueue site via [`InternalReason::mapping`] before the reason is
    /// erased into `DatatypeError::Internal`:
    /// `enqueue()` → `end_transaction()` → `RecoveryAction::RollbackTransaction`
    /// (the unbuffered transaction is rolled back and the datatype stays alive).
    #[error("non-sequential cseq in push buffer")]
    NonSequentialCseq,
    /// The push buffer could not provide the transactions to push (cseq out of range).
    ///
    /// Route: `create_push_pull_pack()` → `do_push_pull()` → `mapping()`
    /// → `RecoveryAction::Disable`.
    #[error("failed to get pushing transactions")]
    GetPushingTransactions,
}

impl InternalReason {
    /// Converts to `DatatypeError::Internal` with the formatted reason message.
    pub(crate) fn into_error(self) -> DatatypeError {
        DatatypeError::Internal(self.to_string())
    }

    /// Converts this reason into its recovery action.
    ///
    /// `into_error()` erases the reason into `DatatypeError::Internal(String)`, so a variant
    /// that needs a routing other than the `Internal` default (`RecoveryAction::Disable`) must
    /// be mapped here, at its creation site, before the erasure. All other variants delegate to
    /// [`DatatypeError::mapping`], which remains the single source of truth per error variant.
    pub(crate) fn mapping(self) -> DatatypeErrorWithAction {
        match self {
            InternalReason::NonSequentialCseq => {
                DatatypeErrorWithAction::new(self.into_error(), RecoveryAction::RollbackTransaction)
            }
            InternalReason::Deserialize(_)
            | InternalReason::ExecuteOperation(_)
            | InternalReason::EventLoop(_)
            | InternalReason::GetPushingTransactions => self.into_error().mapping(),
        }
    }
}

/// Reason a server permanently rejected a datatype operation.
///
/// Carried by [`DatatypeError::ServerRejected`]. New lifecycle operations
/// (e.g., delete, merge) add variants here without touching `DatatypeError` itself.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum ServerRejectReason {
    /// The server refused to create the datatype (e.g., already exists).
    CreateFailed(String),
    /// The requested resource does not exist or has an incompatible type.
    ResourceNotFound(String),
    /// The server-side subscription entry is missing (e.g., server restarted and lost state).
    MissingSubscription(String),
    /// The push violated the wire protocol (e.g., unexpected state transition, type mismatch).
    ProtocolViolation(String),
}

/// Errors that can occur while working with Qortoo datatypes.
///
/// This enum is shared across datatype implementations (e.g., `Counter`) to surface
/// recoverable failures to API callers. Each variant carries a short, human-readable
/// message describing the reason.
///
/// # Equality
/// Two `DatatypeError` values are considered equal if they are the **same variant**,
/// regardless of their message payload. See the custom `PartialEq` implementation.
///
#[non_exhaustive]
#[repr(i32)]
#[derive(Debug, Error, Clone)]
pub enum DatatypeError {
    /// Transaction execution failed.
    ///
    /// Returned when a closure passed to `transaction` returns an error or when the
    /// transactional context cannot be committed. The datatype state is left unchanged
    /// if a rollback succeeds.
    #[error("[DatatypeError] transaction failed: {0}")]
    TransactionFailed(String) = 201,
    /// An internal SDK error that is not caused by user code.
    ///
    /// The message describes the internal failure. This error is not user-actionable;
    /// please report it as a bug.
    #[error("[DatatypeError] internal error: {0}")]
    Internal(String) = 202,
    /// Access denied for a reason other than state or readonly flag (e.g., key not managed by this client).
    #[error("[DatatypeError] disallowed to {0}")]
    Disallowed(String) = 205,
    /// Write rejected because the datatype state does not allow writes.
    #[error("[DatatypeError] not writable: {0}")]
    NotWritable(String) = 206,
    /// Write rejected because the datatype is configured as readonly.
    #[error("[DatatypeError] readonly violation")]
    ReadonlyViolation = 207,

    /// A transient sync failure that warrants a retry with backoff.
    ///
    /// The connectivity backend failed to complete the push-pull exchange.
    /// The datatype state is not changed; the event loop will retry with exponential backoff.
    #[error("[DatatypeError] sync failed: {0}")]
    SyncFailed(String) = 210,
    #[error("[DatatypeError] pushBuffer exceeded max size of memory")]
    PushBufferExceededMaxMemSize = 211,
    /// The server permanently rejected the operation. The datatype transitions to `Disabled`.
    #[error("[DatatypeError] server rejected: {0:?}")]
    ServerRejected(ServerRejectReason) = 213,
}

impl DatatypeError {
    /// Converts this error into its recovery action.
    ///
    /// This is the single source of truth for action routing. Variants handled here:
    /// - `SyncFailed`       — transient connectivity failure → `RetryWithBackOff`
    /// - `Internal`         — fatal SDK-internal fault → `Disable`
    ///   (reason-level overrides live in [`InternalReason::mapping`])
    /// - `ServerRejected`   — server permanently rejected the operation → `Disable`
    /// - `ReadonlyViolation`— server rejected a write from a readonly client → `Disable`
    /// - `PushBufferExceededMaxMemSize` — the transaction cannot be buffered
    ///   → `RollbackTransaction`
    ///
    /// Variants that are returned directly to API callers must never reach this method.
    pub(crate) fn mapping(self) -> DatatypeErrorWithAction {
        match self {
            DatatypeError::SyncFailed(_) => {
                DatatypeErrorWithAction::new(self, RecoveryAction::RetryWithBackOff)
            }
            DatatypeError::Internal(_)
            | DatatypeError::ServerRejected(_)
            | DatatypeError::ReadonlyViolation => {
                DatatypeErrorWithAction::new(self, RecoveryAction::Disable)
            }
            DatatypeError::PushBufferExceededMaxMemSize => {
                DatatypeErrorWithAction::new(self, RecoveryAction::RollbackTransaction)
            }
            // These variants are returned directly to API callers and are never routed through
            // the event loop. Reaching here indicates a misrouted error.
            DatatypeError::TransactionFailed(_)
            | DatatypeError::Disallowed(_)
            | DatatypeError::NotWritable(_) => {
                unreachable!(
                    "variant {:?} must not be routed through DatatypeError::mapping()",
                    self
                )
            }
        }
    }
}

impl PartialEq for DatatypeError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

// Manual impl (not #[derive(Eq)]) because derive would add `ServerRejectReason: Eq` as a where
// bound, which fails since ServerRejectReason does not implement Eq. The unconditional impl is
// sound because the custom PartialEq above is discriminant-only, so reflexivity always holds.
impl Eq for DatatypeError {}

/// How the SDK recovers from a routed datatype error.
///
/// Each variant is a self-consistent recovery policy: it bundles the event-loop scheduling
/// effect and the datatype-lifecycle side effect that must occur together. Contradictory
/// pairings of the two effects (e.g., disabling the datatype while keeping sync scheduled)
/// are unrepresentable by construction.
///
/// Consumers dispatch exhaustively:
/// - event-loop scheduling: `LoopMode` derivation in `event_loop.rs`
/// - datatype side effect: `MutableDatatype::apply_action()`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Notify the `on_error` handler only; no state change and no scheduling change.
    ///
    /// Reserved: no producer yet.
    NotifyOnly,
    /// Transient failure: retry sync with exponential backoff; the datatype is untouched.
    RetryWithBackOff,
    /// Commit-time failure: roll back the pending transaction; sync is unaffected.
    ///
    /// Consumed on the user thread by `TransactionalDatatype::end_transaction()`;
    /// never routed through the event loop.
    RollbackTransaction,
    /// Rebuild the replica: reset local state and re-subscribe from a server snapshot
    /// on the next sync.
    ///
    /// Reserved: no producer yet. WARNING: the reset discards unpushed transactions in the
    /// push buffer — a local data-loss policy must be decided before wiring a producer.
    Resubscribe,
    /// Same as [`RecoveryAction::Resubscribe`], but waits with exponential backoff first
    /// (e.g., the server is temporarily unavailable).
    ///
    /// Reserved: no producer yet. Carries the same data-loss warning as `Resubscribe`.
    ResubscribeWithBackOff,
    /// Permanent failure: stop syncing and disable the datatype; user intervention required.
    Disable,
}

#[derive(Debug)]
pub struct DatatypeErrorWithAction {
    pub error: DatatypeError,
    pub recovery: RecoveryAction,
}

impl DatatypeErrorWithAction {
    pub(crate) fn new(error: DatatypeError, recovery: RecoveryAction) -> Self {
        DatatypeErrorWithAction { error, recovery }
    }
}
