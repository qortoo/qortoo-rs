use std::sync::Arc;

use super::{SNAPSHOT_VERSION, SnapshotCodec, SnapshotReader, write_sized};
use crate::{
    DatatypeError,
    datatypes::crdts::{map_crdt::MapCrdt, timed_node::TimedNode},
    errors::{datatypes::deserialize_error, messages::snapshot as msg},
};

const CONTEXT: &str = msg::MAP_CONTEXT;

impl SnapshotCodec for MapCrdt {
    fn encode_snapshot(&self) -> Box<[u8]> {
        let mut bytes = vec![SNAPSHOT_VERSION];
        bytes.extend_from_slice(&(self.entries.size() as u64).to_le_bytes());
        for (key, entry) in &self.entries {
            write_sized(&mut bytes, key.as_bytes());
            bytes.extend_from_slice(&entry.ts.to_bytes());
            bytes.push(u8::from(entry.v.is_some()));
            if let Some(value) = &entry.v {
                write_sized(&mut bytes, value);
            }
        }
        bytes.into_boxed_slice()
    }

    fn decode_snapshot(bytes: &[u8]) -> Result<Self, DatatypeError> {
        let mut reader = SnapshotReader::new(bytes, CONTEXT);
        reader.read_version(SNAPSHOT_VERSION)?;
        let count = reader.read_u64_le("entry count")?;
        let mut map = Self::default();
        let mut previous_key: Option<Arc<str>> = None;
        for _ in 0..count {
            let key = reader.read_sized("key")?;
            let key = std::str::from_utf8(key)
                .map_err(|_| deserialize_error(CONTEXT, msg::MAP_INVALID_KEY_UTF8))?;
            if previous_key
                .as_deref()
                .is_some_and(|previous| previous >= key)
            {
                return Err(deserialize_error(CONTEXT, msg::MAP_INVALID_KEY_ORDER));
            }
            let key: Arc<str> = key.into();
            let timestamp = reader.read_timestamp("timestamp")?;
            if timestamp.lamport() == 0 {
                return Err(deserialize_error(
                    CONTEXT,
                    msg::MAP_POSITIVE_TIMESTAMP_REQUIRED,
                ));
            }
            let value = match reader.read_u8("state tag")? {
                0 => None,
                1 => Some(Arc::from(reader.read_json()?)),
                _ => return Err(deserialize_error(CONTEXT, msg::MAP_INVALID_STATE_TAG)),
            };
            map.replace(
                key.clone(),
                Some(TimedNode {
                    ts: timestamp,
                    v: value,
                    approved_sseq_upper: None,
                }),
            );
            previous_key = Some(key);
        }
        reader.finish()?;
        Ok(map)
    }
}
