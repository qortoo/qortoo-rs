use thiserror::Error;

use crate::{DatatypeError, errors::messages::sync as msg};

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
    #[error("{prefix}{_0}", prefix = msg::TIMED_OUT_PREFIX)]
    TimedOut(String),
}

impl ConnectivityError {
    pub(crate) fn to_datatype_error(&self) -> DatatypeError {
        match self {
            ConnectivityError::TimedOut(_) => DatatypeError::SyncFailed(self.to_string()),
        }
    }
}
