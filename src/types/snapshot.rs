//! Snapshot writer choices and independent receiver limits.
use crate::defaults::{
    DEFAULT_SNAPSHOT_MAX_INPUT_BYTES, DEFAULT_SNAPSHOT_MAX_MAP_ENTRIES,
    DEFAULT_SNAPSHOT_MAX_RAW_BYTES,
};

/// Binary encoding of a revision-one snapshot payload.
#[cfg_attr(
    not(feature = "snapshot-bitcode"),
    doc = "
```compile_fail
let _ = qortoo::SnapshotCodecKind::Bitcode;
```"
)]
#[cfg_attr(
    not(feature = "snapshot-postcard"),
    doc = "
```compile_fail
let _ = qortoo::SnapshotCodecKind::Postcard;
```"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotCodecKind {
    /// Native bitcode encoding; requires `snapshot-bitcode`.
    #[cfg(feature = "snapshot-bitcode")]
    Bitcode,
    /// Postcard encoding; requires `snapshot-postcard`.
    #[cfg(feature = "snapshot-postcard")]
    Postcard,
}

impl SnapshotCodecKind {
    pub(crate) fn id(self) -> u8 {
        match self {
            #[cfg(feature = "snapshot-bitcode")]
            Self::Bitcode => 1,
            #[cfg(feature = "snapshot-postcard")]
            Self::Postcard => 2,
        }
    }

    pub(crate) fn from_id(id: u8) -> Option<Self> {
        match id {
            #[cfg(feature = "snapshot-bitcode")]
            1 => Some(Self::Bitcode),
            #[cfg(feature = "snapshot-postcard")]
            2 => Some(Self::Postcard),
            _ => None,
        }
    }
}

/// Compression of the complete framed snapshot payload.
#[cfg_attr(
    not(feature = "snapshot-zstd"),
    doc = "
```compile_fail
let _ = qortoo::SnapshotCompression::Zstd1;
```"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SnapshotCompression {
    /// Store the payload without compression.
    None,
    /// A single Zstd frame, written at level 1; requires `snapshot-zstd`.
    #[cfg(feature = "snapshot-zstd")]
    Zstd1,
}

impl SnapshotCompression {
    pub(crate) fn id(self) -> u8 {
        match self {
            Self::None => 0,
            #[cfg(feature = "snapshot-zstd")]
            Self::Zstd1 => 1,
        }
    }

    pub(crate) fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Self::None),
            #[cfg(feature = "snapshot-zstd")]
            1 => Some(Self::Zstd1),
            _ => None,
        }
    }
}

/// Explicit snapshot writer policy. Only enabled capabilities can be selected.
///
/// Reader capabilities are independent of this policy. `Default` requires both
/// `snapshot-bitcode` and `snapshot-zstd` and always selects Bitcode + Zstd1.
#[cfg_attr(
    not(all(feature = "snapshot-bitcode", feature = "snapshot-zstd")),
    doc = "
```compile_fail
let _ = qortoo::SnapshotWriteOptions::default();
```

```compile_fail
let _ = qortoo::LocalConnectivity::new_arc();
```"
)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotWriteOptions {
    /// Encoding used before compression.
    pub codec: SnapshotCodecKind,
    /// Compression applied to the complete payload.
    pub compression: SnapshotCompression,
}

/// Receiver policy, independent of the sender's writer choices.
///
/// Zero rejects any positive size in the corresponding dimension. Limits are inclusive;
/// they bound encoded bytes, raw bytes, and entry count, not peak heap usage.
/// Defaults allow 129 MiB of input, 128 MiB of raw payload, and 1,000,000 Map entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SnapshotDecodeLimits {
    /// Maximum complete snapshot size, including its header, in bytes.
    pub max_input_bytes: usize,
    /// Maximum decompressed payload size in bytes.
    pub max_raw_bytes: usize,
    /// Maximum number of Map entries, including tombstones.
    pub max_map_entries: usize,
}

#[cfg(all(feature = "snapshot-bitcode", feature = "snapshot-zstd"))]
impl Default for SnapshotWriteOptions {
    fn default() -> Self {
        Self {
            codec: SnapshotCodecKind::Bitcode,
            compression: SnapshotCompression::Zstd1,
        }
    }
}

impl Default for SnapshotDecodeLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: DEFAULT_SNAPSHOT_MAX_INPUT_BYTES,
            max_raw_bytes: DEFAULT_SNAPSHOT_MAX_RAW_BYTES,
            max_map_entries: DEFAULT_SNAPSHOT_MAX_MAP_ENTRIES,
        }
    }
}
