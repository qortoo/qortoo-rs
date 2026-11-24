use super::{
    SnapshotCodec, WINDOW_LOG,
    wire::{decode_frame, decode_payload, encode_payload},
};
use crate::{
    SnapshotCodecKind, SnapshotDecodeLimits, SnapshotErrorReason,
    datatypes::crdts::counter_crdt::CounterCrdt,
};

impl SnapshotCodec for CounterCrdt {
    const KIND: u8 = 1;

    fn encode_raw(
        &self,
        codec: SnapshotCodecKind,
        _max_map_entries: usize,
    ) -> Result<Vec<u8>, SnapshotErrorReason> {
        let dto = SnapshotCounter { value: self.value };
        encode_payload!(codec, &dto)
    }

    fn decode_snapshot(
        bytes: &[u8],
        limits: SnapshotDecodeLimits,
    ) -> Result<Self, SnapshotErrorReason> {
        let (codec, raw) = decode_frame(bytes, Self::KIND, limits, WINDOW_LOG)?;
        let dto: SnapshotCounter = decode_payload!(codec, raw.as_ref())?;
        Ok(CounterCrdt { value: dto.value })
    }
}

#[cfg_attr(feature = "snapshot-bitcode", derive(bitcode::Encode, bitcode::Decode))]
#[cfg_attr(
    feature = "snapshot-postcard",
    derive(serde::Serialize, serde::Deserialize)
)]
pub(super) struct SnapshotCounter {
    pub(super) value: i64,
}
