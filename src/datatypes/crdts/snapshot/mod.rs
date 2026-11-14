mod counter;
mod map;
mod reader;
mod variable;

use reader::SnapshotReader;

use crate::DatatypeError;

pub(super) const SNAPSHOT_VERSION: u8 = 1;

fn write_sized(bytes: &mut Vec<u8>, value: &[u8]) {
    let length = u64::try_from(value.len()).expect("snapshot field length exceeds u64");
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(value);
}

/// Converts a concrete CRDT state to and from its snapshot representation.
///
/// Decoding returns a fully validated replacement so callers can preserve the
/// current state when an invalid snapshot is received.
pub(crate) trait SnapshotCodec: Sized {
    fn encode_snapshot(&self) -> Box<[u8]>;

    fn decode_snapshot(snapshot: &[u8]) -> Result<Self, DatatypeError>;
}
