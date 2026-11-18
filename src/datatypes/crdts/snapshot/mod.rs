#[cfg(feature = "snapshot-zstd")]
mod compression;
mod counter;
mod map;
mod variable;
mod wire;

#[cfg(test)]
mod tests;

use crate::{
    SnapshotDecodeLimits, SnapshotErrorReason, SnapshotWriteOptions,
    defaults::SNAPSHOT_MAX_ZSTD_WINDOW_LOG as WINDOW_LOG,
};

fn buffer_with_capacity(size: usize) -> Result<Vec<u8>, SnapshotErrorReason> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| SnapshotErrorReason::ResourceExhausted)?;
    Ok(bytes)
}

/// Converts a concrete CRDT state to and from its snapshot representation.
/// Decoding validates a complete candidate before the caller replaces current state.
pub(crate) trait SnapshotCodec: Sized {
    fn encode_snapshot(
        &self,
        options: SnapshotWriteOptions,
    ) -> Result<Box<[u8]>, SnapshotErrorReason>;
    fn decode_snapshot(
        snapshot: &[u8],
        limits: SnapshotDecodeLimits,
    ) -> Result<Self, SnapshotErrorReason>;
}
