use thiserror::Error;

use crate::{ConnectivityError, DatatypeState, errors::push_pull::ServerPushPullError};

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
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum DatatypeError {
    /// Transaction execution failed.
    ///
    /// Returned when a closure passed to `transaction` returns an error or when the
    /// transactional context cannot be committed. The datatype state is left unchanged
    /// if a rollback succeeds.
    #[error("[DatatypeError] failed to do transaction: {0}")]
    FailedTransaction(String) = 201,
    /// Deserialization from bytes failed.
    ///
    /// Returned when decoding a datatype, operation, or internal state from a byte
    /// sequence is not possible (e.g., invalid length, unexpected format, or version
    /// mismatch).
    #[error("[DatatypeError] failed to deserialize: {0}")]
    FailedToDeserialize(String) = 202,
    /// Applying a local operation failed.
    ///
    /// Returned when an operation cannot be executed in the current state (e.g.,
    /// unsupported operation kind, precondition violations, or internal invariants
    /// not satisfied).
    #[error("[DatatypeError] failed to execute operation: {0}")]
    FailedToExecuteOperation(String) = 203,
    #[error("[DatatypeError] failure in EventLoop")]
    FailedInEventLoop(String) = 204,
    #[error("[DatatypeError] disallowed to {0}")]
    Disallowed(String) = 205,

    #[error("[DatatypeError] failed in connectivity: {0}")]
    FailedInConnectivity(ConnectivityError) = 210,
    #[error("[DatatypeError] pushBuffer exceeded max size of memory")]
    PushBufferExceededMaxMemSize = 211,
    #[error("[DatatypeError] failed by server push-pull error: {0}")]
    FailedByServerPushPullError(ServerPushPullError) = 212,
    #[error("[DatatypeError] failed by protocol violation: {0}")]
    FailedByProtocolViolation(String) = 213,
    #[error("[DatatypeError] failed to create datatype: {0}")]
    FailedToCreate(String) = 214,
    #[error("[DatatypeError] failed to subscribe datatype: {0}")]
    FailedToSubscribe(String) = 215,
    #[error("[DatatypeError] an operation of nonsequential cseq is enqueued into PushBuffer")]
    NonSequentialCseq = 216,
    #[error("[DatatypeError] failed to get pushing transactions")]
    FailedToGetPushingTransactions = 217,
}

impl DatatypeError {
    pub(crate) fn mapping(self) -> DatatypeErrorWithActions {
        let event_loop_action = match &self {
            DatatypeError::FailedInConnectivity(_) => EventLoopAction::BackOff,
            DatatypeError::NonSequentialCseq => EventLoopAction::Normal,
            DatatypeError::FailedTransaction(_)
            | DatatypeError::FailedToDeserialize(_)
            | DatatypeError::FailedToExecuteOperation(_)
            | DatatypeError::FailedInEventLoop(_)
            | DatatypeError::Disallowed(_)
            | DatatypeError::PushBufferExceededMaxMemSize
            | DatatypeError::FailedByServerPushPullError(_)
            | DatatypeError::FailedByProtocolViolation(_)
            | DatatypeError::FailedToCreate(_)
            | DatatypeError::FailedToSubscribe(_)
            | DatatypeError::FailedToGetPushingTransactions => EventLoopAction::PauseSync,
        };
        let datatype_action = match &self {
            DatatypeError::FailedInConnectivity(_) => DatatypeAction::Normal,
            DatatypeError::NonSequentialCseq => DatatypeAction::Reset,
            DatatypeError::FailedTransaction(_)
            | DatatypeError::FailedToDeserialize(_)
            | DatatypeError::FailedToExecuteOperation(_)
            | DatatypeError::FailedInEventLoop(_)
            | DatatypeError::Disallowed(_)
            | DatatypeError::PushBufferExceededMaxMemSize
            | DatatypeError::FailedByServerPushPullError(_)
            | DatatypeError::FailedByProtocolViolation(_)
            | DatatypeError::FailedToCreate(_)
            | DatatypeError::FailedToSubscribe(_)
            | DatatypeError::FailedToGetPushingTransactions => DatatypeAction::Disable,
        };

        DatatypeErrorWithActions::new(self, event_loop_action, datatype_action)
    }
}

// impl PartialEq for DatatypeError {
//     fn eq(&self, other: &Self) -> bool {
//         std::mem::discriminant(self) == std::mem::discriminant(other)
//     }
// }

pub enum EventLoopAction {
    Normal,
    BackOff,
    PauseSync,
}

#[derive(Debug, PartialEq)]
pub enum DatatypeAction {
    Normal,
    // set the datatype state to 'Due_To_SubscribeOrCreate' so that the datatype should restart sync.
    Restart,
    // set the datatype state to 'Disabled' so that the datatype should stop sync.
    Disable,
    // set the datatype state to 'Subscribed' and reset the datatype with the snapshot sent by the server.
    Reset,
}

impl From<DatatypeState> for DatatypeAction {
    fn from(value: DatatypeState) -> Self {
        match value {
            DatatypeState::Creating => DatatypeAction::Restart,
            DatatypeState::Subscribing => DatatypeAction::Restart,
            DatatypeState::SubscribingOrCreating => DatatypeAction::Restart,
            DatatypeState::Subscribed => DatatypeAction::Normal,
            DatatypeState::Unsubscribing => DatatypeAction::Normal,
            DatatypeState::Deleting => DatatypeAction::Normal,
            DatatypeState::Disabled => DatatypeAction::Disable,
        }
    }
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

#[cfg(test)]
mod tests_datatypes {
    use rstest::rstest;

    use crate::{DatatypeState, errors::datatypes::DatatypeAction};

    #[rstest]
    #[case(DatatypeState::Creating, DatatypeAction::Restart)]
    #[case(DatatypeState::Subscribing, DatatypeAction::Restart)]
    #[case(DatatypeState::SubscribingOrCreating, DatatypeAction::Restart)]
    #[case(DatatypeState::Subscribed, DatatypeAction::Normal)]
    #[case(DatatypeState::Unsubscribing, DatatypeAction::Normal)]
    #[case(DatatypeState::Deleting, DatatypeAction::Normal)]
    #[case(DatatypeState::Disabled, DatatypeAction::Disable)]
    fn can_convert_datatype_state_into_action(
        #[case] state: DatatypeState,
        #[case] expected_action: DatatypeAction,
    ) {
        let datatype_action: DatatypeAction = state.into();
        assert_eq!(datatype_action, expected_action);
    }
}
