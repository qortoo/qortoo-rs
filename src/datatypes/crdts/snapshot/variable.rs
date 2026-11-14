use super::{SNAPSHOT_VERSION, SnapshotCodec, SnapshotReader, write_sized};
use crate::{
    DatatypeError,
    datatypes::crdts::variable_crdt::{INITIAL_VALUE, VariableCrdt, VariableState},
    errors::{datatypes::deserialize_error, messages::snapshot as snapshot_msg},
    types::timestamp::Timestamp,
};

impl SnapshotCodec for VariableCrdt {
    fn encode_snapshot(&self) -> Box<[u8]> {
        let mut serialized = vec![SNAPSHOT_VERSION];
        serialized.extend_from_slice(&self.timestamp().to_bytes());
        write_sized(&mut serialized, self.value());
        serialized.into_boxed_slice()
    }

    fn decode_snapshot(snapshot: &[u8]) -> Result<Self, DatatypeError> {
        let mut reader = SnapshotReader::new(snapshot, snapshot_msg::VARIABLE_CONTEXT);
        reader.read_version(SNAPSHOT_VERSION)?;
        let timestamp = reader.read_timestamp(snapshot_msg::TIMESTAMP_FIELD)?;
        let value = reader.read_json()?;
        validate_snapshot_state(value, &timestamp)?;
        let winning = VariableState::new(value, &timestamp);

        reader.finish()?;
        Ok(Self { winning })
    }
}

fn validate_snapshot_state(value: &[u8], timestamp: &Timestamp) -> Result<(), DatatypeError> {
    if timestamp.lamport() > 0 {
        return Ok(());
    }
    if timestamp.is_initial() && value == INITIAL_VALUE {
        return Ok(());
    }
    Err(deserialize_error(
        snapshot_msg::VARIABLE_CONTEXT,
        snapshot_msg::INVALID_INITIAL_VARIABLE,
    ))
}
