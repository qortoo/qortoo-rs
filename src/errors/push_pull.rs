use thiserror::Error;

use crate::{
    ConnectivityError, DatatypeError, DatatypeState,
    errors::datatypes::{DatatypeAction, DatatypeErrorWithActions, EventLoopAction},
};

pub(crate) const CLIENT_PUSHPULL_ERR_MSG_NO_SNAPSHOT: &str = "no snapshot operation";

#[non_exhaustive]
#[repr(i32)]
#[derive(Debug, Error /*PartialEq*/, Eq, Clone)]
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
                    DatatypeState::DueToCreate => DatatypeError::FailedToCreate(msg.to_owned()),
                    DatatypeState::DueToSubscribe => {
                        DatatypeError::FailedToSubscribe(msg.to_owned())
                    }
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

#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ClientPushPullError {
    #[error("[ClientPushPullError] pushBuffer exceeded max size of memory")]
    ExceedMaxMemSize,
    #[error("[ClientPushPullError] an operation of nonsequential cseq is enqueued into PushBuffer")]
    NonSequentialCseq,
    #[error("[ClientPushPullError] failed to get after")]
    FailToGetPushingTransactions,
    #[error("[ClientPushPullError] failed in Connectivity: {0}")]
    FailedInConnectivity(ConnectivityError),
    #[error("[ClientPushPullError] failed with protocol violation: {0}")]
    FailedWithProtocolViolation(String),
}

impl ClientPushPullError {
    pub fn mapping(self) -> DatatypeErrorWithActions {
        match self {
            ClientPushPullError::ExceedMaxMemSize => todo!(),
            ClientPushPullError::NonSequentialCseq => DatatypeErrorWithActions::new(
                DatatypeError::FailedByClientPushPullError(self),
                EventLoopAction::Normal,
                DatatypeAction::Reset,
            ),
            ClientPushPullError::FailToGetPushingTransactions => DatatypeErrorWithActions::new(
                DatatypeError::FailedByClientPushPullError(self),
                EventLoopAction::PauseSync,
                DatatypeAction::Disable,
            ),
            ClientPushPullError::FailedInConnectivity(_) => DatatypeErrorWithActions::new(
                DatatypeError::FailedByClientPushPullError(self),
                EventLoopAction::BackOff,
                DatatypeAction::Normal,
            ),
            ClientPushPullError::FailedWithProtocolViolation(_) => DatatypeErrorWithActions::new(
                DatatypeError::FailedByClientPushPullError(self),
                EventLoopAction::PauseSync,
                DatatypeAction::Disable,
            ),
        }
    }
}

impl From<ConnectivityError> for ClientPushPullError {
    fn from(ce: ConnectivityError) -> Self {
        ClientPushPullError::FailedInConnectivity(ce)
    }
}
