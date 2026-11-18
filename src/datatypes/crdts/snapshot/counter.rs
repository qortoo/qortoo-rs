use super::{
    SnapshotCodec, WINDOW_LOG,
    wire::{decode_frame, decode_payload, encode_frame, encode_payload},
};
use crate::{
    SnapshotDecodeLimits, SnapshotErrorReason, SnapshotWriteOptions,
    datatypes::crdts::counter_crdt::CounterCrdt,
};

impl SnapshotCodec for CounterCrdt {
    fn encode_snapshot(
        &self,
        options: SnapshotWriteOptions,
    ) -> Result<Box<[u8]>, SnapshotErrorReason> {
        let dto = SnapshotCounter { value: self.value };
        encode_frame(
            1,
            options,
            &encode_payload!(options.codec, &dto)?,
            WINDOW_LOG,
        )
    }

    fn decode_snapshot(
        bytes: &[u8],
        limits: SnapshotDecodeLimits,
    ) -> Result<Self, SnapshotErrorReason> {
        let (codec, raw) = decode_frame(bytes, 1, limits, WINDOW_LOG)?;
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
