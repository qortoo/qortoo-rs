use std::sync::Arc;

use crate::{
    CollectionInfo, SnapshotWriteOptions, datatypes::Crdt, errors::push_pull::PushPullError,
    operations::transaction::Transaction,
};

/// Server-only settings for a collection in [`LocalConnectivity`](crate::LocalConnectivity).
#[derive(Debug, Clone, Copy)]
pub struct LocalCollectionOptions {
    /// Codec and compression used to write snapshots for this collection.
    pub snapshot_write_options: SnapshotWriteOptions,
    /// Checks incoming writes against the collection's key and value policy. Defaults to `true`.
    pub validate_writes: bool,
    /// Checks the merged logical state size after each transaction. Defaults to `true`.
    pub validate_state_size: bool,
}

impl LocalCollectionOptions {
    /// Uses the given snapshot writer and enables both server policy checks.
    pub fn new(snapshot_write_options: SnapshotWriteOptions) -> Self {
        Self {
            snapshot_write_options,
            validate_writes: true,
            validate_state_size: true,
        }
    }
}

#[cfg(all(feature = "snapshot-bitcode", feature = "snapshot-zstd"))]
impl Default for LocalCollectionOptions {
    fn default() -> Self {
        Self::new(SnapshotWriteOptions::default())
    }
}

pub(super) struct LocalCollectionConfig {
    pub(super) info: Arc<CollectionInfo>,
    pub(super) options: LocalCollectionOptions,
}

impl LocalCollectionConfig {
    pub(super) fn new(info: CollectionInfo, options: LocalCollectionOptions) -> Self {
        Self {
            info: Arc::new(info),
            options,
        }
    }

    pub(super) fn validate_transaction_writes(
        &self,
        tx: &Transaction,
    ) -> Result<(), PushPullError> {
        if self.options.validate_writes {
            for op in tx.iter() {
                self.info
                    .validate_write(&op.body)
                    .map_err(PushPullError::CollectionPolicyViolation)?;
            }
        }
        Ok(())
    }

    pub(super) fn state_size_before(&self, crdt: &Crdt) -> Option<u64> {
        (self.options.validate_state_size && self.info.max_state_bytes != 0)
            .then(|| crdt.logical_state_bytes())
    }

    pub(super) fn validate_state_change(
        &self,
        before: Option<u64>,
        crdt: &Crdt,
    ) -> Result<(), PushPullError> {
        if let Some(before) = before {
            self.info
                .validate_state_change(before, crdt.logical_state_bytes())
                .map_err(PushPullError::CollectionPolicyViolation)?;
        }
        Ok(())
    }
}
