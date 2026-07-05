use thiserror::Error;

use crate::DatatypeError;

/// Errors related to connectivity operations.
///
/// # Equality
/// Two `ConnectivityError` values are considered equal if they have the same
/// variant and message content.
#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum ConnectivityError {
    /// The connectivity backend did not respond within the expected time.
    ///
    /// This is a transient error. The event loop will retry with exponential backoff.
    #[error("[ConnectivityError] connection timed out: {_0}")]
    TimedOut(String),
}

impl ConnectivityError {
    pub(crate) fn to_datatype_error(&self) -> DatatypeError {
        match self {
            ConnectivityError::TimedOut(_) => DatatypeError::SyncFailed(self.to_string()),
        }
    }
}
