use thiserror::Error;

use crate::{DatatypeError, ServerRejectReason, errors::messages::sync as msg};

/// Wire-level error set by the responder in `PushPullPack.error`.
///
/// In the push-pull protocol the responder (the server side) fills this field when it
/// rejects or fails to process a request. The client converts it into a [`DatatypeError`]
/// via [`PushPullError::to_datatype_error`]; variant names mirror [`ServerRejectReason`]
/// where a counterpart exists.
#[non_exhaustive]
#[repr(i32)]
#[derive(Debug, Error, Eq, Clone)]
pub enum PushPullError {
    #[error("[E{code}] {prefix}{0}", code = self.code(), prefix = msg::PROTOCOL_VIOLATION_PREFIX)]
    ProtocolViolation(String) = 301,
    #[error("[E{code}] {}", msg::READONLY_VIOLATION, code = self.code())]
    ReadonlyViolation = 302,
    #[error("[E{code}] {prefix}{0}", code = self.code(), prefix = msg::CREATE_FAILED_PREFIX)]
    CreateFailed(String) = 303,
    /// The requested resource does not exist or has an incompatible type.
    #[error("[E{code}] {prefix}{0}", code = self.code(), prefix = msg::RESOURCE_NOT_FOUND_PREFIX)]
    ResourceNotFound(String) = 304,
    #[error("[E{code}] {prefix}{0}", code = self.code(), prefix = msg::MISSING_SUBSCRIPTION_PREFIX)]
    MissingSubscription(String) = 305,
    /// The server encountered a temporary internal error and could not process the request.
    ///
    /// The client should retry with exponential backoff. This maps to
    /// [`DatatypeError::SyncFailed`] → `RecoveryAction::RetryWithBackOff`.
    #[error("[E{code}] {prefix}{0}", code = self.code(), prefix = msg::SERVER_INTERNAL_ERROR_PREFIX)]
    ServerInternalError(String) = 306,
}

impl PushPullError {
    /// Returns the stable numeric diagnostic code shared with language bindings.
    ///
    /// Use this value for programmatic decisions rather than parsing the message.
    pub const fn code(&self) -> i32 {
        match self {
            Self::ProtocolViolation(_) => 301,
            Self::ReadonlyViolation => 302,
            Self::CreateFailed(_) => 303,
            Self::ResourceNotFound(_) => 304,
            Self::MissingSubscription(_) => 305,
            Self::ServerInternalError(_) => 306,
        }
    }

    pub fn to_datatype_error(&self) -> DatatypeError {
        match self {
            PushPullError::ProtocolViolation(msg) => {
                DatatypeError::ServerRejected(ServerRejectReason::ProtocolViolation(msg.to_owned()))
            }
            PushPullError::ReadonlyViolation => DatatypeError::ReadonlyViolation,
            PushPullError::CreateFailed(msg) => {
                DatatypeError::ServerRejected(ServerRejectReason::CreateFailed(msg.to_owned()))
            }
            PushPullError::ResourceNotFound(msg) => {
                DatatypeError::ServerRejected(ServerRejectReason::ResourceNotFound(msg.to_owned()))
            }
            PushPullError::MissingSubscription(msg) => DatatypeError::ServerRejected(
                ServerRejectReason::MissingSubscription(msg.to_owned()),
            ),
            PushPullError::ServerInternalError(msg) => DatatypeError::SyncFailed(msg.to_owned()),
        }
    }
}

impl PartialEq for PushPullError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}
