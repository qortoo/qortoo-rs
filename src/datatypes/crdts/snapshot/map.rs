use std::{
    collections::{BTreeMap, BTreeSet, btree_map::Entry::Vacant},
    sync::Arc,
};

use super::{
    SnapshotCodec, WINDOW_LOG,
    wire::{
        Codec, Result as WireResult, decode_frame, decode_payload, encode_frame, encode_payload,
        read_size, take_bytes, write_size,
    },
};
use crate::{
    SnapshotDecodeLimits, SnapshotErrorReason, SnapshotWriteOptions,
    datatypes::crdts::{map_crdt::MapCrdt, timed_node::TimedNode},
    types::{timestamp::Timestamp, uid::Cuid},
    utils::json::validate_json_str,
};

impl SnapshotCodec for MapCrdt {
    fn encode_snapshot(
        &self,
        options: SnapshotWriteOptions,
    ) -> Result<Box<[u8]>, SnapshotErrorReason> {
        let mut raw = Vec::new();
        write_size(&mut raw, self.entries.size())?;
        let cuid_indices = self.write_cuid_table(&mut raw)?;
        self.write_map_blocks(options.codec, &cuid_indices, &mut raw)?;
        encode_frame(3, options, &raw, WINDOW_LOG)
    }

    fn decode_snapshot(
        bytes: &[u8],
        limits: SnapshotDecodeLimits,
    ) -> Result<Self, SnapshotErrorReason> {
        let (codec, raw) = decode_frame(bytes, 3, limits, WINDOW_LOG)?;
        let mut cursor = raw.as_ref();
        let count = read_size(&mut cursor)?;
        if count > limits.max_map_entries {
            return Err(SnapshotErrorReason::LimitExceeded);
        }
        let cuids = read_cuid_table(&mut cursor, count)?;
        let state = Self::read_map_blocks(&mut cursor, codec, count, &cuids)?;
        if !cursor.is_empty() {
            return Err(SnapshotErrorReason::Invalid);
        }
        Ok(state)
    }
}

#[cfg_attr(feature = "snapshot-bitcode", derive(bitcode::Encode, bitcode::Decode))]
#[cfg_attr(
    feature = "snapshot-postcard",
    derive(serde::Serialize, serde::Deserialize)
)]
#[derive(Clone, PartialEq, Eq)]
pub(super) struct SnapshotMapRow<'a> {
    pub(super) key: &'a str,
    pub(super) lamport: u64,
    pub(super) cuid_index: u32,
    #[cfg_attr(feature = "snapshot-postcard", serde(borrow))]
    pub(super) value: Option<&'a str>,
}

pub(super) type SnapshotMapBlock<'a> = [SnapshotMapRow<'a>; 32];

pub(super) fn padding() -> SnapshotMapRow<'static> {
    SnapshotMapRow {
        key: "",
        lamport: 0,
        cuid_index: 0,
        value: None,
    }
}

impl MapCrdt {
    fn write_cuid_table(&self, raw: &mut Vec<u8>) -> WireResult<BTreeMap<[u8; 16], u32>> {
        let mut indices = BTreeMap::new();
        let mut table = Vec::new();
        for (_, entry) in &self.entries {
            let cuid_bytes = entry.ts.cuid().to_bytes();
            if let Vacant(entry) = indices.entry(cuid_bytes) {
                let index = u32::try_from(table.len()).map_err(|_| SnapshotErrorReason::Invalid)?;
                entry.insert(index);
                table.push(cuid_bytes);
            }
        }
        raw.extend_from_slice(
            &u32::try_from(table.len())
                .map_err(|_| SnapshotErrorReason::Invalid)?
                .to_le_bytes(),
        );
        for cuid_bytes in table {
            raw.extend_from_slice(&cuid_bytes);
        }
        Ok(indices)
    }

    fn write_map_blocks(
        &self,
        codec: Codec,
        cuid_indices: &BTreeMap<[u8; 16], u32>,
        raw: &mut Vec<u8>,
    ) -> WireResult<()> {
        let mut entries = self.entries.iter();
        loop {
            let mut block: SnapshotMapBlock = std::array::from_fn(|_| padding());
            let mut used = 0;
            for row in &mut block {
                let Some((key, entry)) = entries.next() else {
                    break;
                };
                *row = SnapshotMapRow {
                    key,
                    lamport: entry.ts.lamport(),
                    cuid_index: cuid_indices[&entry.ts.cuid().to_bytes()],
                    value: entry
                        .v
                        .as_deref()
                        .map(validate_json_str)
                        .transpose()
                        .map_err(|_| SnapshotErrorReason::Invalid)?,
                };
                used += 1;
            }
            if used == 0 {
                break;
            }
            let bytes = encode_payload!(codec, &block)?;
            write_size(raw, bytes.len())?;
            raw.extend(bytes);
        }
        Ok(())
    }

    fn read_map_blocks(
        cursor: &mut &[u8],
        codec: Codec,
        count: usize,
        cuids: &[Cuid],
    ) -> WireResult<Self> {
        let mut state = Self::default();
        let mut previous: Option<Arc<str>> = None;
        let blocks = count.div_ceil(32);
        for i in 0..blocks {
            let len = read_size(cursor)?;
            let bytes = take_bytes(cursor, len)?;
            let block: SnapshotMapBlock = decode_payload!(codec, bytes)?;
            let used = (count - i * 32).min(32);
            for (j, row) in block.into_iter().enumerate() {
                if j >= used {
                    if row != padding() {
                        return Err(SnapshotErrorReason::Invalid);
                    }
                    continue;
                }
                if row.lamport == 0 || previous.as_deref().is_some_and(|p| p >= row.key) {
                    return Err(SnapshotErrorReason::Invalid);
                }
                let node = row.to_timed_node(cuids)?;
                let key: Arc<str> = row.key.into();
                state.replace(key.clone(), Some(node));
                previous = Some(key);
            }
        }
        Ok(state)
    }
}

impl SnapshotMapRow<'_> {
    fn to_timed_node(&self, cuids: &[Cuid]) -> WireResult<TimedNode> {
        let cuid = cuids
            .get(self.cuid_index as usize)
            .ok_or(SnapshotErrorReason::Invalid)?;
        let value = self
            .value
            .map(|value| validate_json_str(value.as_bytes()).map(|_| Arc::from(value.as_bytes())))
            .transpose()
            .map_err(|_| SnapshotErrorReason::Invalid)?;
        Ok(TimedNode {
            ts: Timestamp::new(self.lamport, cuid),
            v: value,
            approved_sseq_upper: None,
        })
    }
}

fn read_cuid_table(cursor: &mut &[u8], entry_count: usize) -> WireResult<Vec<Cuid>> {
    let cuid_count = u32::from_le_bytes(take_bytes(cursor, 4)?.try_into().unwrap()) as usize;
    if cuid_count > entry_count {
        return Err(SnapshotErrorReason::Invalid);
    }
    let table = take_bytes(
        cursor,
        cuid_count
            .checked_mul(16)
            .ok_or(SnapshotErrorReason::Invalid)?,
    )?;
    let mut seen = BTreeSet::new();
    let mut cuids = Vec::new();
    cuids
        .try_reserve_exact(cuid_count)
        .map_err(|_| SnapshotErrorReason::ResourceExhausted)?;
    for cuid_bytes in table.chunks_exact(16) {
        let cuid = Cuid::from_bytes(cuid_bytes).map_err(|_| SnapshotErrorReason::Invalid)?;
        if !seen.insert(cuid.clone()) {
            return Err(SnapshotErrorReason::Invalid);
        }
        cuids.push(cuid);
    }
    Ok(cuids)
}
