use thiserror::Error;

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
    #[error("[ClientError] invalid collection name: {0}")]
    InvalidCollectionName(String) = 100,

    /// Subscribe or Create Datatype failed.
    ///
    /// Returned when a request to subscribe or create a datatype is
    /// incompatible with an existing instance for the same key (for
    /// example, mismatched type or datatype state).
    #[error("[ClientError] cannot subscribe or create datatype: {0}")]
    FailedToSubscribeOrCreateDatatype(String) = 101,
}

impl PartialEq for ClientError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

pub(crate) const CLIENT_ERROR_MSG_COLLECTION_NAME: &str = "invalid collection name: '{}' - Collection name must be 1-47 characters, start with a letter or underscore, contain only alphanumeric characters and '.', '_', '~', '-', and must not start with 'system.' or contain '.system.'";
pub(crate) const CLIENT_ERROR_MSG_DATATYPE_KEY: &str = "invalid datatype key: '{}' - Key must not be empty, must not contain null characters (\\0), must not exceed 255 bytes in length, and must not start with '$'.";

#[cfg(test)]
mod tests_client_error {
    use dyn_fmt::AsStrFormatExt;
    use tracing::info;

    use crate::errors::clients::CLIENT_ERROR_MSG_COLLECTION_NAME;

    #[test]
    fn can_use_error_msg_format() {
        let invalid_collection_name = "invalid::collection::name";
        let err_msg = CLIENT_ERROR_MSG_COLLECTION_NAME.format(&[invalid_collection_name]);
        info!("{err_msg}");
        assert!(err_msg.contains(invalid_collection_name));
    }
}
