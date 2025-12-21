use thiserror::Error;

use crate::ConnectivityError;

#[repr(i32)]
#[derive(Debug, Error, Eq)]
pub enum ServerPushPullError {
    #[error("[ServerPushPullError] illegal push request - {0}")]
    IllegalPushRequest(String) = 301,
    #[error("[ServerPushPull] fail to create - {0}")]
    FailedToCreate(String) = 302,
    #[error("[ServerPushPull] fail to subscribe - {0}")]
    FailedToSubscribe(String) = 303,
}

impl PartialEq for ServerPushPullError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

#[allow(dead_code)]
pub enum CaseAfterPushPullError {
    // The case that can be resolved with backoff retry
    BackOff,
    // The case that can be resolved by resetting the datatype
    Reset,
    // The case that any illegal case happens
    Abort,
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum ClientPushPullError {
    #[error("[ClientPushPullError] pushBuffer exceeded max size of memory")]
    ExceedMaxMemSize,
    #[error("[ClientPushPullError] an operation of nonsequential cseq is enqued into PushBuffer")]
    NonSequentialCseq,
    #[error("[ClientPushPullError] failed to get after")]
    FailToGetAfter,
    #[error("[ClientPushPullError] failed in Connectivity: {0}")]
    FailedInConnectivity(ConnectivityError),
    #[error("[ClientPushPullError] failed and abort datatype: {0}")]
    FailedAndAbort(String),
}

impl ClientPushPullError {
    #[allow(dead_code)]
    fn how_to_deal_with_error(&self) -> CaseAfterPushPullError {
        match self {
            ClientPushPullError::ExceedMaxMemSize => todo!(),
            ClientPushPullError::NonSequentialCseq => CaseAfterPushPullError::Abort,
            ClientPushPullError::FailToGetAfter => CaseAfterPushPullError::Abort,
            ClientPushPullError::FailedInConnectivity(_ce) => {
                todo!();
            }
            ClientPushPullError::FailedAndAbort(_) => todo!(),
        }
    }
}

impl From<ConnectivityError> for ClientPushPullError {
    fn from(ce: ConnectivityError) -> Self {
        ClientPushPullError::FailedInConnectivity(ce)
    }
}
