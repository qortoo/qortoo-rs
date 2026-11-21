use crate::{
    ClientError, SnapshotWriteOptions,
    defaults::{DEFAULT_MAX_SNAPSHOT_BYTES, DEFAULT_MAX_TRANSMISSION_BYTES},
    errors::messages::client as msg,
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
    /// Writer options for this collection's snapshots.
    ///
    /// On creation, `None` selects the backend's default. In the open response,
    /// `None` means the backend does not report any writer options.
    pub snapshot_write_options: Option<SnapshotWriteOptions>,
    /// Byte limit for a batch of transactions. A single transaction may exceed it.
    pub max_transmission_bytes: u64,
}

impl CollectionInfo {
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
            snapshot_write_options: None,
            max_transmission_bytes: DEFAULT_MAX_TRANSMISSION_BYTES,
        }
    }
}
