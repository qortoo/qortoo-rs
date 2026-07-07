use thiserror::Error;

use crate::{DatatypeError, ServerRejectReason};

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
    #[error("[PushPullError] protocol violation - {0}")]
    ProtocolViolation(String) = 301,
    #[error("[PushPullError] readonly client attempted write operation")]
    ReadonlyViolation = 302,
    #[error("[PushPullError] fail to create - {0}")]
    CreateFailed(String) = 303,
    /// The requested resource does not exist or has an incompatible type.
    #[error("[PushPullError] resource not found - {0}")]
    ResourceNotFound(String) = 304,
    #[error("[PushPullError] client's datatype is not subscribed on this server - {0}")]
    MissingSubscription(String) = 305,
    /// The server encountered a temporary internal error and could not process the request.
    ///
    /// The client should retry with exponential backoff. This maps to
    /// [`DatatypeError::SyncFailed`] → `RecoveryAction::RetryWithBackOff`.
    #[error("[PushPullError] server internal error - {0}")]
    ServerInternalError(String) = 306,
}

impl PushPullError {
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
