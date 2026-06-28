use thiserror::Error;


/// Internal SDK error reason, used by the event loop for action routing.
///
/// Not exposed to users. Internal callers use this to construct `DatatypeError::Internal`
/// via `into_error()`.
#[derive(Debug, Error)]
pub(crate) enum InternalReason {
    #[error("deserialize: {0}")]
    Deserialize(String),
    #[error("execute operation: {0}")]
    ExecuteOperation(String),
    #[error("event loop: {0}")]
    EventLoop(String),
    /// Cseq was non-sequential in the push buffer.
    /// Requires `Normal + Rollback` instead of the default `StopSync + Disable`.
    #[error("non-sequential cseq in push buffer")]
    NonSequentialCseq,
    #[error("failed to get pushing transactions")]
    GetPushingTransactions,
}

impl InternalReason {
    /// Converts to `DatatypeError::Internal` with the formatted reason message.
    pub(crate) fn into_error(self) -> DatatypeError {
        DatatypeError::Internal(self.to_string())
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
    /// Converts this error into event-loop routing actions.
    ///
    /// This is the single source of truth for action routing. Variants handled here:
    /// - `SyncFailed`       — transient connectivity failure → retry with backoff
    /// - `Internal`         — fatal SDK-internal fault → stop sync, disable
    /// - `ServerRejected`   — server permanently rejected the operation → stop sync, disable
    /// - `ReadonlyViolation`— server rejected a write from a readonly client → stop sync, disable
    ///
    /// Variants that are returned directly to API callers must never reach this method.
    pub(crate) fn mapping(self) -> DatatypeErrorWithActions {
        match self {
            DatatypeError::SyncFailed(_) => {
                DatatypeErrorWithActions::new(self, EventLoopAction::BackOff, DatatypeAction::Normal)
            }
            DatatypeError::Internal(_)
            | DatatypeError::ServerRejected(_)
            | DatatypeError::ReadonlyViolation => {
                DatatypeErrorWithActions::new(self, EventLoopAction::StopSync, DatatypeAction::Disable)
            }
            // These variants are returned directly to API callers and are never routed through
            // the event loop. Reaching here indicates a misrouted error.
            DatatypeError::TransactionFailed(_)
            | DatatypeError::Disallowed(_)
            | DatatypeError::NotWritable(_)
            | DatatypeError::PushBufferExceededMaxMemSize => {
                unreachable!("variant {:?} must not be routed through DatatypeError::mapping()", self)
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

pub enum EventLoopAction {
    Normal,
    BackOff,
    StopSync,
}

#[derive(Debug, PartialEq)]
pub enum DatatypeAction {
    Normal,
    // set the datatype state to 'SubscribingOrCreating' so that the datatype should restart sync.
    Restart,
    // set the datatype state to 'Disabled' so that the datatype should stop sync.
    Disable,
    // rolls back the in-progress transaction, leaving the datatype state unchanged.
    Rollback,
}

pub struct DatatypeErrorWithActions {
    pub error: DatatypeError,
    pub event_loop_action: EventLoopAction,
    pub datatype_action: DatatypeAction,
}

impl DatatypeErrorWithActions {
    pub(crate) fn new(
        error: DatatypeError,
        event_loop_action: EventLoopAction,
        datatype_action: DatatypeAction,
    ) -> Self {
        DatatypeErrorWithActions {
            error,
            event_loop_action,
            datatype_action,
        }
    }
}

