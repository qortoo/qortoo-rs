use std::sync::Arc;

use qortoo::{LocalConnectivity, SnapshotCodecKind, SnapshotCompression, SnapshotWriteOptions};

pub fn local_connectivity() -> Arc<LocalConnectivity> {
    #[cfg(feature = "snapshot-bitcode")]
    let codec = SnapshotCodecKind::Bitcode;
    #[cfg(not(feature = "snapshot-bitcode"))]
    let codec = SnapshotCodecKind::Postcard;
    #[cfg(feature = "snapshot-zstd")]
    let compression = SnapshotCompression::Zstd1;
    #[cfg(not(feature = "snapshot-zstd"))]
    let compression = SnapshotCompression::None;
    LocalConnectivity::new_arc_with_snapshot_options(SnapshotWriteOptions { codec, compression })
}
