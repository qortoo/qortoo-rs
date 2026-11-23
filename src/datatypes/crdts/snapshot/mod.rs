#[cfg(feature = "snapshot-zstd")]
mod compression;
mod counter;
mod map;
mod variable;
mod wire;

#[cfg(test)]
mod tests;

use crate::{
    SnapshotCodecKind, SnapshotDecodeLimits, SnapshotErrorReason, SnapshotWriteOptions,
    defaults::SNAPSHOT_MAX_ZSTD_WINDOW_LOG as WINDOW_LOG,
};

fn buffer_with_capacity(size: usize) -> Result<Vec<u8>, SnapshotErrorReason> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(size)
        .map_err(|_| SnapshotErrorReason::ResourceExhausted)?;
    Ok(bytes)
}

#[cfg(test)]
pub(super) const UNLIMITED: SnapshotDecodeLimits = SnapshotDecodeLimits {
    max_input_bytes: usize::MAX,
    max_raw_bytes: usize::MAX,
    max_map_entries: usize::MAX,
};

/// Converts a concrete CRDT state to and from its snapshot representation.
/// Decoding validates a complete candidate before the caller replaces current state.
pub(crate) trait SnapshotCodec: Sized {
    const KIND: u8;
    fn encode_raw(
        &self,
        codec: SnapshotCodecKind,
        max_map_entries: usize,
    ) -> Result<Vec<u8>, SnapshotErrorReason>;

    #[cfg(test)]
    fn encode_snapshot(
        &self,
        options: SnapshotWriteOptions,
    ) -> Result<Box<[u8]>, SnapshotErrorReason> {
        self.encode_snapshot_with_limits(options, UNLIMITED)
    }

    fn encode_snapshot_with_limits(
        &self,
        options: SnapshotWriteOptions,
        limits: SnapshotDecodeLimits,
    ) -> Result<Box<[u8]>, SnapshotErrorReason> {
        wire::encode_frame(
            Self::KIND,
            options,
            &self.encode_raw(options.codec, limits.max_map_entries)?,
            limits,
            WINDOW_LOG,
        )
    }
    fn decode_snapshot(
        snapshot: &[u8],
        limits: SnapshotDecodeLimits,
    ) -> Result<Self, SnapshotErrorReason>;
}
