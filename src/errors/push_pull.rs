use thiserror::Error;

use crate::{
    DatatypeError,
    errors::datatypes::{DatatypeAction, DatatypeErrorWithActions, EventLoopAction},
};

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
    #[error("[PushPullError] fail to subscribe - {0}")]
    FailedToSubscribe(String) = 304,
    #[error("[PushPullError] client's datatype is not subscribed on this server - {0}")]
    FailedByMissingSubscription(String) = 305,
}

impl ServerPushPullError {
    pub fn mapping(&self) -> DatatypeErrorWithActions {
        match self {
            ServerPushPullError::FailedByIllegalRequest(msg) => DatatypeErrorWithActions::new(
                DatatypeError::FailedByProtocolViolation(msg.to_owned()),
                EventLoopAction::PauseSync,
                DatatypeAction::Disable,
            ),
            ServerPushPullError::FailedByReadonlyRestriction => DatatypeErrorWithActions::new(
                DatatypeError::Disallowed("readonly client attempted write operation".to_owned()),
                EventLoopAction::PauseSync,
                DatatypeAction::Disable,
            ),
            ServerPushPullError::FailedToCreate(msg) => DatatypeErrorWithActions::new(
                DatatypeError::FailedToCreate(msg.to_owned()),
                EventLoopAction::PauseSync,
                DatatypeAction::Disable,
            ),
            ServerPushPullError::FailedToSubscribe(msg) => DatatypeErrorWithActions::new(
                DatatypeError::FailedToSubscribe(msg.to_owned()),
                EventLoopAction::PauseSync,
                DatatypeAction::Disable,
            ),
            ServerPushPullError::FailedByMissingSubscription(msg) => DatatypeErrorWithActions::new(
                DatatypeError::FailedToSubscribe(msg.to_owned()),
                EventLoopAction::PauseSync,
                DatatypeAction::Restart,
            ),
        }
    }
}

impl PartialEq for ServerPushPullError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}
