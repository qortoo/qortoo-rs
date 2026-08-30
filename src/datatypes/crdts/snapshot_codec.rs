use crate::DatatypeError;

/// Converts a concrete CRDT state to and from its snapshot representation.
///
/// Decoding returns a fully validated replacement so callers can preserve the
/// current state when an invalid snapshot is received.
pub(crate) trait SnapshotCodec: Sized {
    fn encode_snapshot(&self) -> Box<[u8]>;

    fn decode_snapshot(snapshot: &[u8]) -> Result<Self, DatatypeError>;
}
