use thiserror::Error;

use crate::{DatatypeError, ServerRejectReason};

#[non_exhaustive]
#[repr(i32)]
#[derive(Debug, Error, Eq, Clone)]
pub enum ServerPushPullError {
    #[error("[PushPullError] illegal push request - {0}")]
    FailedByIllegalRequest(String) = 301,
    #[error("[PushPullError] readonly client attempted write operation")]
    FailedByReadonlyRestriction = 302,
    #[error("[PushPullError] fail to create - {0}")]
    FailedToCreate(String) = 303,
    /// The requested resource does not exist or has an incompatible type.
    #[error("[PushPullError] resource not found - {0}")]
    FailedByResourceNotFound(String) = 304,
    #[error("[PushPullError] client's datatype is not subscribed on this server - {0}")]
    FailedByMissingSubscription(String) = 305,
    /// The server encountered a temporary internal error and could not process the request.
    ///
    /// The client should retry with exponential backoff. This maps to
    /// [`DatatypeError::SyncFailed`] so the event loop applies `BackOff + Normal`.
    #[error("[PushPullError] server internal error - {0}")]
    FailedByServerInternalError(String) = 306,
}

impl ServerPushPullError {
    pub fn to_datatype_error(&self) -> DatatypeError {
        match self {
            ServerPushPullError::FailedByIllegalRequest(msg) => {
                DatatypeError::ServerRejected(ServerRejectReason::ProtocolViolation(msg.to_owned()))
            }
            ServerPushPullError::FailedByReadonlyRestriction => DatatypeError::ReadonlyViolation,
            ServerPushPullError::FailedToCreate(msg) => {
                DatatypeError::ServerRejected(ServerRejectReason::CreateFailed(msg.to_owned()))
            }
            ServerPushPullError::FailedByResourceNotFound(msg) => {
                DatatypeError::ServerRejected(ServerRejectReason::ResourceNotFound(msg.to_owned()))
            }
            ServerPushPullError::FailedByMissingSubscription(msg) => {
                DatatypeError::ServerRejected(ServerRejectReason::MissingSubscription(msg.to_owned()))
            }
            ServerPushPullError::FailedByServerInternalError(msg) => {
                DatatypeError::SyncFailed(msg.to_owned())
            }
        }
    }
}

impl PartialEq for ServerPushPullError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}
