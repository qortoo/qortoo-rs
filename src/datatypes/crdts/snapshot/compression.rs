//! A single ordinary Zstd frame, with bounded output and native error classification.
use ::zstd::zstd_safe::{
    self as zstd, CParameter, DParameter, InBuffer, OutBuffer, zstd_sys::ZSTD_ErrorCode,
};

use super::buffer_with_capacity;
use crate::SnapshotErrorReason;

fn native_error(code: zstd::ErrorCode) -> SnapshotErrorReason {
    if zstd::is_error_kind(
        code,
        ZSTD_ErrorCode::ZSTD_error_frameParameter_windowTooLarge,
    ) {
        SnapshotErrorReason::LimitExceeded
    } else if zstd::is_error_kind(code, ZSTD_ErrorCode::ZSTD_error_memory_allocation) {
        SnapshotErrorReason::ResourceExhausted
    } else {
        SnapshotErrorReason::Invalid
    }
}

pub(super) fn encode(raw: &[u8], window_log: u32) -> Result<Vec<u8>, SnapshotErrorReason> {
    if window_log == 0 {
        return Err(SnapshotErrorReason::Invalid);
    }
    let mut context = zstd::CCtx::try_create().ok_or(SnapshotErrorReason::ResourceExhausted)?;
    for parameter in [
        CParameter::CompressionLevel(1),
        CParameter::WindowLog(window_log),
        CParameter::ChecksumFlag(true),
        CParameter::ContentSizeFlag(true),
    ] {
        context.set_parameter(parameter).map_err(native_error)?;
    }
    let mut output = buffer_with_capacity(zstd::compress_bound(raw.len()))?;
    context.compress2(&mut output, raw).map_err(native_error)?;
    Ok(output)
}

pub(super) fn decode(
    input: &[u8],
    declared: usize,
    window_log: u32,
) -> Result<Vec<u8>, SnapshotErrorReason> {
    if window_log == 0 {
        return Err(SnapshotErrorReason::Invalid);
    }
    // Require ordinary Zstd magic; frame-size detection also accepts skippable frames.
    if !input.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return Err(SnapshotErrorReason::Invalid);
    }
    if zstd::find_frame_compressed_size(input).map_err(native_error)? != input.len() {
        return Err(SnapshotErrorReason::Invalid);
    }
    if zstd::get_dict_id_from_frame(input).is_some() {
        return Err(SnapshotErrorReason::Unsupported);
    }
    if zstd::get_frame_content_size(input)
        .map_err(|_| SnapshotErrorReason::Invalid)?
        .is_some_and(|size| size != declared as u64)
    {
        return Err(SnapshotErrorReason::Invalid);
    }
    let mut context = zstd::DCtx::try_create().ok_or(SnapshotErrorReason::ResourceExhausted)?;
    context
        .set_parameter(DParameter::WindowLogMax(window_log))
        .map_err(native_error)?;
    let mut output = buffer_with_capacity(declared)?;
    output.resize(declared, 0);
    let mut source = InBuffer::around(input);
    let mut target = OutBuffer::around(output.as_mut_slice());
    loop {
        let before = (source.pos(), target.pos());
        let remaining = context
            .decompress_stream(&mut target, &mut source)
            .map_err(native_error)?;
        if remaining == 0 {
            if source.pos() != input.len() || target.pos() != declared {
                return Err(SnapshotErrorReason::Invalid);
            }
            break;
        }
        if target.pos() == declared {
            // Consume the epilogue/checksum even when the declared output is already full.
            let mut extra = [0u8; 1];
            let mut overflow = OutBuffer::around(&mut extra[..]);
            let remaining = context
                .decompress_stream(&mut overflow, &mut source)
                .map_err(native_error)?;
            if overflow.pos() != 0 || remaining != 0 || source.pos() != input.len() {
                return Err(SnapshotErrorReason::Invalid);
            }
            break;
        }
        if before == (source.pos(), target.pos()) {
            return Err(SnapshotErrorReason::Invalid);
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    const WINDOW_LOG: u32 = 20;

    fn frame(raw: &[u8], checksum: bool, known_size: bool) -> Vec<u8> {
        let mut encoder = ::zstd::stream::write::Encoder::new(Vec::new(), 1).unwrap();
        encoder.window_log(WINDOW_LOG).unwrap();
        encoder.include_checksum(checksum).unwrap();
        if known_size {
            encoder
                .set_pledged_src_size(Some(raw.len() as u64))
                .unwrap();
        }
        encoder.write_all(raw).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn can_require_one_complete_frame_and_exact_output() {
        let raw = b"snapshot repeated content ".repeat(1000);
        for known_size in [false, true] {
            for checksum in [false, true] {
                let bytes = frame(&raw, checksum, known_size);
                assert_eq!(decode(&bytes, raw.len(), WINDOW_LOG).unwrap(), raw);
                for declared in [raw.len() - 1, raw.len() + 1] {
                    assert_eq!(
                        decode(&bytes, declared, WINDOW_LOG),
                        Err(SnapshotErrorReason::Invalid)
                    );
                }
                for end in 0..bytes.len() {
                    assert!(decode(&bytes[..end], raw.len(), WINDOW_LOG).is_err());
                }
                for suffix in [vec![0], frame(b"", false, false), bytes.clone()] {
                    let mut extra = bytes.clone();
                    extra.extend(suffix);
                    assert_eq!(
                        decode(&extra, raw.len(), WINDOW_LOG),
                        Err(SnapshotErrorReason::Invalid)
                    );
                }
                if checksum {
                    let mut damaged = bytes;
                    *damaged.last_mut().unwrap() ^= 1;
                    assert_eq!(
                        decode(&damaged, raw.len(), WINDOW_LOG),
                        Err(SnapshotErrorReason::Invalid)
                    );
                }
            }
        }
        let skippable = [0x50, 0x2a, 0x4d, 0x18, 0, 0, 0, 0];
        assert_eq!(
            decode(&skippable, 0, WINDOW_LOG),
            Err(SnapshotErrorReason::Invalid)
        );
    }

    #[test]
    fn can_distinguish_window_limits_and_dictionary_requirements() {
        for (descriptor, expected) in [
            (0x50, Ok(vec![])),
            (0x70, Err(SnapshotErrorReason::LimitExceeded)),
        ] {
            let bytes = [0x28, 0xb5, 0x2f, 0xfd, 0, descriptor, 1, 0, 0];
            assert_eq!(decode(&bytes, 0, WINDOW_LOG), expected);
            assert_eq!(decode(&bytes, 0, 24).unwrap(), Vec::<u8>::new());
        }
        let dictionary_id = [0x28, 0xb5, 0x2f, 0xfd, 1, 0x50, 1, 1, 0, 0];
        assert_eq!(
            decode(&dictionary_id, 0, WINDOW_LOG),
            Err(SnapshotErrorReason::Unsupported)
        );
        let dictionary = b"abcdef-ghijkl-mnopqr-stuvwx-yz0123-456789".repeat(100);
        let raw = dictionary.repeat(10);
        let mut encoder =
            ::zstd::stream::write::Encoder::with_dictionary(Vec::new(), 1, &dictionary).unwrap();
        encoder.include_dictid(false).unwrap();
        encoder.write_all(&raw).unwrap();
        let bytes = encoder.finish().unwrap();
        assert!(zstd::get_dict_id_from_frame(&bytes).is_none());
        assert_eq!(
            decode(&bytes, raw.len(), WINDOW_LOG),
            Err(SnapshotErrorReason::Invalid)
        );
    }

    #[test]
    fn can_write_frames_with_a_compatible_window_and_content_size() {
        for raw in [vec![], vec![b'x'; 1], vec![b'x'; 3 * 1024 * 1024]] {
            let bytes = encode(&raw, WINDOW_LOG).unwrap();
            assert_eq!(
                zstd::get_frame_content_size(&bytes).unwrap(),
                Some(raw.len() as u64)
            );
            assert_eq!(decode(&bytes, raw.len(), WINDOW_LOG).unwrap(), raw);
            let mut damaged = bytes;
            *damaged.last_mut().unwrap() ^= 1;
            assert_eq!(
                decode(&damaged, raw.len(), WINDOW_LOG),
                Err(SnapshotErrorReason::Invalid)
            );
        }
        for window in [0, 9, 32, u32::MAX] {
            assert_eq!(encode(b"", window), Err(SnapshotErrorReason::Invalid));
            assert_eq!(
                decode(&frame(b"", false, true), 0, window),
                Err(SnapshotErrorReason::Invalid)
            );
        }
    }
}
