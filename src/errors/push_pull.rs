use thiserror::Error;

use crate::{
    DatatypeError, DatatypeState,
    errors::datatypes::{DatatypeErrorWithActions, EventLoopAction},
};

#[non_exhaustive]
#[repr(i32)]
#[derive(Debug, Error, Eq, Clone)]
pub enum ServerPushPullError {
    #[error("[ServerPushPullError] illegal push request - {0}")]
    IllegalPushRequest(String) = 301,
    #[error("[ServerPushPull] fail to create - {0}")]
    FailedToCreate(String) = 302,
    #[error("[ServerPushPull] fail to subscribe - {0}")]
    FailedToSubscribe(String) = 303,
}

impl ServerPushPullError {
    pub fn mapping(
        &self,
        old_state: DatatypeState,
        new_state: DatatypeState,
    ) -> DatatypeErrorWithActions {
        match self {
            ServerPushPullError::IllegalPushRequest(msg) => {
                let data_err = match old_state {
                    DatatypeState::Creating => DatatypeError::FailedToCreate(msg.to_owned()),
                    DatatypeState::Subscribing => DatatypeError::FailedToSubscribe(msg.to_owned()),
                    _ => DatatypeError::FailedByServerPushPullError(self.clone()),
                };
                DatatypeErrorWithActions::new(
                    data_err,
                    EventLoopAction::PauseSync,
                    new_state.into(),
                )
            }
            ServerPushPullError::FailedToCreate(msg) => DatatypeErrorWithActions::new(
                DatatypeError::FailedToCreate(msg.to_owned()),
                EventLoopAction::PauseSync,
                new_state.into(),
            ),
            ServerPushPullError::FailedToSubscribe(msg) => DatatypeErrorWithActions::new(
                DatatypeError::FailedToSubscribe(msg.to_owned()),
                EventLoopAction::PauseSync,
                new_state.into(),
            ),
        }
    }
}

impl PartialEq for ServerPushPullError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}
