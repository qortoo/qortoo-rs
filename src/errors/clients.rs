use thiserror::Error;

use crate::errors::messages::client as msg;

/// Errors related to client-side operations and datatype management.
///
/// # Equality
/// Two `ClientError` values are considered equal if they are the **same variant**,
/// regardless of their message payload. See the custom `PartialEq` implementation.
///
#[non_exhaustive]
#[repr(i32)]
#[derive(Debug, Error)]
pub enum ClientError {
    /// Invalid collection name provided.
    ///
    /// Returned when attempting to create a client with a collection name
    /// that does not meet validation requirements.
    #[error("[E{code}] {prefix}{0}", code = self.code(), prefix = msg::INVALID_COLLECTION_NAME_PREFIX)]
    InvalidCollectionName(String) = 100,

    /// Subscribe or Create Datatype failed.
    ///
    /// Returned when a request to subscribe or create a datatype is
    /// incompatible with an existing instance for the same key (for
    /// example, mismatched type or datatype state).
    #[error("[E{code}] {prefix}{0}", code = self.code(), prefix = msg::SUBSCRIBE_OR_CREATE_FAILED_PREFIX)]
    FailedToSubscribeOrCreateDatatype(String) = 101,
}

impl PartialEq for ClientError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl ClientError {
    /// Returns the stable numeric diagnostic code shared with language bindings.
    ///
    /// Use this value for programmatic decisions rather than parsing the message.
    pub const fn code(&self) -> i32 {
        match self {
            Self::InvalidCollectionName(_) => 100,
            Self::FailedToSubscribeOrCreateDatatype(_) => 101,
        }
    }
}

#[cfg(test)]
mod tests_client_error {
    use crate::Client;

    #[test]
    fn can_describe_collection_name_requirements() {
        let error = Client::builder("invalid::collection::name", "alias")
            .build()
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            "[E100] invalid collection name: 'invalid::collection::name': collection name must be 1-47 bytes, start with an ASCII letter or underscore, contain only ASCII letters, digits, '.', '_', '~', or '-', and must not start with 'system.' or contain '.system.'"
        );
    }
}
