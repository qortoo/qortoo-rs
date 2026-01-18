use thiserror::Error;

/// Errors related to connectivity operations.
///
/// These errors occur when interacting with the underlying synchronization
/// backend or when resources cannot be found.
///
/// # Equality
/// Two `ConnectivityError` values are considered equal if they have the same
/// variant and message content.
#[non_exhaustive]
#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConnectivityError {
    /// The requested resource was not found.
    ///
    /// Returned when attempting to access a datatype or resource that
    /// does not exist in the connectivity backend.
    #[error("[ConnectivityError] the demanded resource is not found: {_0}")]
    ResourceNotFound(String),
}
