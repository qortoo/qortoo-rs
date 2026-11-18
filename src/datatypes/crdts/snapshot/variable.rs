use super::{
    SnapshotCodec, WINDOW_LOG,
    wire::{decode_frame, decode_payload, encode_frame, encode_payload},
};
use crate::{
    SnapshotDecodeLimits, SnapshotErrorReason, SnapshotWriteOptions,
    datatypes::crdts::variable_crdt::VariableCrdt,
    types::{timestamp::Timestamp, uid::Cuid},
};

impl SnapshotCodec for VariableCrdt {
    fn encode_snapshot(
        &self,
        options: SnapshotWriteOptions,
    ) -> Result<Box<[u8]>, SnapshotErrorReason> {
        let dto = SnapshotVariable {
            lamport: self.timestamp().lamport(),
            cuid: self.timestamp().cuid().to_bytes(),
            value: self.validate_state().ok_or(SnapshotErrorReason::Invalid)?,
        };
        encode_frame(
            2,
            options,
            &encode_payload!(options.codec, &dto)?,
            WINDOW_LOG,
        )
    }

    fn decode_snapshot(
        bytes: &[u8],
        limits: SnapshotDecodeLimits,
    ) -> Result<Self, SnapshotErrorReason> {
        let (codec, raw) = decode_frame(bytes, 2, limits, WINDOW_LOG)?;
        let dto: SnapshotVariable = decode_payload!(codec, raw.as_ref())?;
        let timestamp = Timestamp::new(
            dto.lamport,
            &Cuid::from_bytes(&dto.cuid).map_err(|_| SnapshotErrorReason::Invalid)?,
        );
        let candidate = VariableCrdt::new(dto.value.as_bytes(), &timestamp);
        candidate
            .validate_state()
            .ok_or(SnapshotErrorReason::Invalid)?;
        Ok(candidate)
    }
}

#[cfg_attr(feature = "snapshot-bitcode", derive(bitcode::Encode, bitcode::Decode))]
#[cfg_attr(
    feature = "snapshot-postcard",
    derive(serde::Serialize, serde::Deserialize)
)]
pub(super) struct SnapshotVariable<'a> {
    pub(super) lamport: u64,
    pub(super) cuid: [u8; 16],
    pub(super) value: &'a str,
}
