//! Revision-one snapshot framing. Callers must supply limits; no product defaults live here.
use std::borrow::Cow;

use super::buffer_with_capacity;
#[cfg(feature = "snapshot-zstd")]
use super::compression;
pub(super) use crate::SnapshotCodecKind as Codec;
use crate::{
    SnapshotCompression, SnapshotDecodeLimits as Limits, SnapshotErrorReason, SnapshotWriteOptions,
};

pub(super) type Result<T> = std::result::Result<T, SnapshotErrorReason>;

pub(super) const HEADER_BYTES: usize = 24;

macro_rules! encode_payload {
    ($codec:expr, $value:expr) => {{
        use $crate::{SnapshotErrorReason, datatypes::crdts::snapshot::wire::Codec};
        let result: std::result::Result<std::vec::Vec<u8>, SnapshotErrorReason> = match $codec {
            #[cfg(feature = "snapshot-bitcode")]
            Codec::Bitcode => Ok(bitcode::encode($value)),
            #[cfg(feature = "snapshot-postcard")]
            Codec::Postcard => {
                postcard::to_allocvec($value).map_err(|_| SnapshotErrorReason::Invalid)
            }
        };
        result
    }};
}

macro_rules! decode_payload {
    ($codec:expr, $bytes:expr) => {{
        use $crate::{SnapshotErrorReason, datatypes::crdts::snapshot::wire::Codec};
        match $codec {
            #[cfg(feature = "snapshot-bitcode")]
            Codec::Bitcode => bitcode::decode($bytes).map_err(|_| SnapshotErrorReason::Invalid),
            #[cfg(feature = "snapshot-postcard")]
            Codec::Postcard => postcard::take_from_bytes($bytes)
                .map_err(|_| SnapshotErrorReason::Invalid)
                .and_then(|(value, rest)| {
                    if rest.is_empty() {
                        Ok(value)
                    } else {
                        Err(SnapshotErrorReason::Invalid)
                    }
                }),
        }
    }};
}

pub(super) use decode_payload;
pub(super) use encode_payload;

pub(super) fn take_bytes<'a>(input: &mut &'a [u8], len: usize) -> Result<&'a [u8]> {
    let (value, rest) = input
        .split_at_checked(len)
        .ok_or(SnapshotErrorReason::Invalid)?;
    *input = rest;
    Ok(value)
}

pub(super) fn read_size(input: &mut &[u8]) -> Result<usize> {
    usize::try_from(u64::from_le_bytes(
        take_bytes(input, 8)?.try_into().unwrap(),
    ))
    .map_err(|_| SnapshotErrorReason::Invalid)
}

pub(super) fn write_size(out: &mut Vec<u8>, size: usize) -> Result<()> {
    out.extend_from_slice(
        &u64::try_from(size)
            .map_err(|_| SnapshotErrorReason::Invalid)?
            .to_le_bytes(),
    );
    Ok(())
}

/// Frames an encoded payload using a supported writer policy.
pub(super) fn encode_frame(
    kind: u8,
    options: SnapshotWriteOptions,
    raw: &[u8],
    limits: Limits,
    window_log: u32,
) -> Result<Box<[u8]>> {
    #[cfg(not(feature = "snapshot-zstd"))]
    let _ = window_log;
    if raw.len() > limits.max_raw_bytes {
        return Err(SnapshotErrorReason::LimitExceeded);
    }
    let payload = match options.compression {
        SnapshotCompression::None => Cow::Borrowed(raw),
        #[cfg(feature = "snapshot-zstd")]
        SnapshotCompression::Zstd1 => Cow::Owned(compression::encode(raw, window_log)?),
    };
    let frame_len = HEADER_BYTES
        .checked_add(payload.len())
        .ok_or(SnapshotErrorReason::LimitExceeded)?;
    if frame_len > limits.max_input_bytes {
        return Err(SnapshotErrorReason::LimitExceeded);
    }
    let mut out = buffer_with_capacity(frame_len)?;
    out.extend_from_slice(b"QTSN");
    out.extend_from_slice(&[1, kind, options.codec.id(), options.compression.id()]);
    write_size(&mut out, payload.len())?;
    write_size(&mut out, raw.len())?;
    out.extend_from_slice(&payload);
    Ok(out.into_boxed_slice())
}

pub(super) fn decode_frame(
    input: &[u8],
    kind: u8,
    limits: Limits,
    window_log: u32,
) -> Result<(Codec, Cow<'_, [u8]>)> {
    #[cfg(not(feature = "snapshot-zstd"))]
    let _ = window_log;
    if input.len() > limits.max_input_bytes {
        return Err(SnapshotErrorReason::LimitExceeded);
    }
    let mut cursor = input;
    let header = take_bytes(&mut cursor, HEADER_BYTES)?;
    if &header[..4] != b"QTSN" {
        return Err(SnapshotErrorReason::Invalid);
    }
    if header[4] != 1 || !(1..=3).contains(&header[5]) {
        return Err(SnapshotErrorReason::Unsupported);
    }
    if header[5] != kind {
        return Err(SnapshotErrorReason::Invalid);
    }
    let codec = Codec::from_id(header[6]).ok_or(SnapshotErrorReason::Unsupported)?;
    let compression =
        SnapshotCompression::from_id(header[7]).ok_or(SnapshotErrorReason::Unsupported)?;
    let mut lengths = &header[8..];
    let stored = read_size(&mut lengths)?;
    let raw = read_size(&mut lengths)?;
    if raw > limits.max_raw_bytes {
        return Err(SnapshotErrorReason::LimitExceeded);
    }
    if stored != cursor.len() {
        return Err(SnapshotErrorReason::Invalid);
    }
    let payload = match compression {
        SnapshotCompression::None => {
            if stored != raw {
                return Err(SnapshotErrorReason::Invalid);
            }
            Cow::Borrowed(cursor)
        }
        #[cfg(feature = "snapshot-zstd")]
        SnapshotCompression::Zstd1 => Cow::Owned(compression::decode(cursor, raw, window_log)?),
    };
    Ok((codec, payload))
}
