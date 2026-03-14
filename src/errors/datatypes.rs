use thiserror::Error;

use crate::{
    ConnectivityError,
    errors::{BoxedError, push_pull::ClientPushPullError},
};

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
#[derive(Debug, Error)]
pub enum DatatypeError {
    #[error("[DatatypeError] failed to create datatype: {0}")]
    FailedToCreate(String) = 200,
    #[error("[DatatypeError] failed to create datatype: {0}")]
    FailedInConnectivity(ConnectivityError) = 201,
    /// Transaction execution failed.
    ///
    /// Returned when a closure passed to `transaction` returns an error or when the
    /// transactional context cannot be committed. The datatype state is left unchanged
    /// if a rollback succeeds.
    #[error("[DatatypeError] failed to do transaction: {0}")]
    FailedTransaction(BoxedError) = 202,
    /// Deserialization from bytes failed.
    ///
    /// Returned when decoding a datatype, operation, or internal state from a byte
    /// sequence is not possible (e.g., invalid length, unexpected format, or version
    /// mismatch).
    #[error("[DatatypeError] failed to deserialize: {0}")]
    FailedToDeserialize(String) = 203,
    /// Applying a local operation failed.
    ///
    /// Returned when an operation cannot be executed in the current state (e.g.,
    /// unsupported operation kind, precondition violations, or internal invariants
    /// not satisfied).
    #[error("[DatatypeError] failed to execute operation: {0}")]
    FailedToExecuteOperation(String) = 204,
    #[error("[DatatypeError] failure in EventLoop")]
    FailedInEventLoop(BoxedError) = 205,
    #[error("[DatatypeError] disallowed to {0}")]
    Disallowed(String) = 206,
    #[error("[DatatypeError] failed to push and pull: {0}")]
    FailedToPushPull(ClientPushPullError) = 207,
}

impl PartialEq for DatatypeError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

pub enum EventLoopAction {
    Normal,
    BackOff,
    PauseSync,
}

pub enum DatatypeAction {
    Normal,
    Reset,
    Disable,
    Recovery,
}

pub struct DatatypeErrorWithActions {
    pub error: DatatypeError,
    pub event_loop_action: EventLoopAction,
    pub datatype_action: DatatypeAction,
}

impl DatatypeErrorWithActions {
    pub fn new(
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
