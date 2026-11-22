use crate::{
    ClientError,
    defaults::{DEFAULT_MAX_SNAPSHOT_BYTES, DEFAULT_MAX_TRANSMISSION_BYTES},
    errors::messages::{client as msg, datatype as write_msg},
    operations::body::OperationBody,
};

/// Information about a collection returned by the backend when a client opens.
///
/// A client stores this information on its first successful open.
///
/// Limits are inclusive. A zero state, key, or value limit disables that check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionInfo {
    /// Maximum logical state size per datatype, including Map tombstones.
    pub max_state_bytes: u64,
    /// Maximum UTF-8 size of an individual Map key.
    pub max_key_bytes: u64,
    /// Maximum size of the stored, uncompressed JSON for an individual Map value.
    pub max_value_bytes: u64,
    /// Map keys disallowed by exact, whole-string match.
    pub forbidden_keys: Vec<String>,
    /// Maximum encoded snapshot size including its header.
    pub max_snapshot_bytes: usize,
    /// Byte limit for a batch of transactions. A single transaction may exceed it.
    pub max_transmission_bytes: u64,
}

impl CollectionInfo {
    pub(crate) fn validate_write(&self, body: &OperationBody) -> Result<(), String> {
        if let OperationBody::MapPut(put) = body {
            let key_bytes = put.key.len() as u64;
            let value_bytes = put.value.len() as u64;
            if self.max_key_bytes != 0 && key_bytes > self.max_key_bytes {
                return Err(write_msg::collection_byte_limit(
                    "max_key_bytes",
                    key_bytes,
                    self.max_key_bytes,
                ));
            }
            if self.max_value_bytes != 0 && value_bytes > self.max_value_bytes {
                return Err(write_msg::collection_byte_limit(
                    "max_value_bytes",
                    value_bytes,
                    self.max_value_bytes,
                ));
            }
            if self.forbidden_keys.contains(&put.key) {
                return Err(write_msg::COLLECTION_FORBIDDEN_KEY.to_owned());
            }
        }
        Ok(())
    }

    pub(crate) fn validate_state_change(&self, before: u64, after: u64) -> Result<(), String> {
        if self.max_state_bytes != 0 && after > self.max_state_bytes && after >= before {
            return Err(write_msg::collection_byte_limit(
                "max_state_bytes",
                after,
                self.max_state_bytes,
            ));
        }
        Ok(())
    }

    /// Validates the configured byte limits.
    pub(crate) fn validate_limits(&self) -> Result<(), ClientError> {
        for (field, bytes, minimum, allows_zero) in [
            (
                "max_snapshot_bytes",
                self.max_snapshot_bytes as u64,
                1_000,
                false,
            ),
            (
                "max_transmission_bytes",
                self.max_transmission_bytes,
                1_000,
                false,
            ),
            ("max_state_bytes", self.max_state_bytes, 10_000, true),
            ("max_key_bytes", self.max_key_bytes, 10, true),
            ("max_value_bytes", self.max_value_bytes, 10, true),
        ] {
            if bytes < minimum && !(allows_zero && bytes == 0) {
                return Err(ClientError::InvalidCollectionLimits(
                    msg::collection_limit_too_low(field, minimum, allows_zero),
                ));
            }
        }
        Ok(())
    }
}

impl Default for CollectionInfo {
    fn default() -> Self {
        Self {
            max_state_bytes: 0,
            max_key_bytes: 0,
            max_value_bytes: 0,
            forbidden_keys: Vec::new(),
            max_snapshot_bytes: DEFAULT_MAX_SNAPSHOT_BYTES,
            max_transmission_bytes: DEFAULT_MAX_TRANSMISSION_BYTES,
        }
    }
}
