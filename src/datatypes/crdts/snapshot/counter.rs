use super::SnapshotCodec;
use crate::{
    DatatypeError,
    datatypes::crdts::counter_crdt::CounterCrdt,
    errors::{datatypes::deserialize_error, messages::snapshot as snapshot_msg},
};

const SNAPSHOT_LEN: usize = size_of::<i64>();

impl SnapshotCodec for CounterCrdt {
    fn encode_snapshot(&self) -> Box<[u8]> {
        Box::new(self.value.to_le_bytes())
    }

    fn decode_snapshot(snapshot: &[u8]) -> Result<Self, DatatypeError> {
        let bytes: [u8; SNAPSHOT_LEN] = snapshot.try_into().map_err(|_| {
            deserialize_error(
                snapshot_msg::COUNTER_CONTEXT,
                snapshot_msg::length_mismatch(SNAPSHOT_LEN, snapshot.len()),
            )
        })?;
        Ok(Self {
            value: i64::from_le_bytes(bytes),
        })
    }
}
