use thiserror::Error;

use crate::errors::{BoxedError, connectivity::ConnectivityError};

/// Errors that can occur while working with SyncYam datatypes.
///
/// This enum is shared across datatype implementations (e.g., `Counter`) to surface
/// recoverable failures to API callers. Each variant carries a short, human-readable
/// message describing the reason.
///
/// # Equality
/// Two `DatatypeError` values are considered equal if they are the **same variant**,
/// regardless of their message payload. See the custom `PartialEq` implementation.
///
#[repr(i32)]
#[derive(Debug, Error)]
pub enum DatatypeError {
    #[error("[DatatypeError] failed to build datatype: {0}")]
    FailedBuildDatatype(String) = 201,
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
    FailureInEventLoop(BoxedError) = 205,
    #[error("[DatatypeError] not allowed to write")]
    FailedToWrite(String) = 206,
    #[error("[DatatypeError] failed to push and pull: {0}")]
    FailedToPushPull(ConnectivityError) = 207,
}

impl PartialEq for DatatypeError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}
