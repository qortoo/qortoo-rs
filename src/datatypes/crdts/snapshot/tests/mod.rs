use std::{borrow::Cow, sync::Arc};

use super::{
    SnapshotCodec, buffer_with_capacity,
    counter::SnapshotCounter,
    map::{SnapshotMapBlock, SnapshotMapRow, padding},
    variable::SnapshotVariable,
    wire::{Codec, HEADER_BYTES, decode_frame, encode_frame, encode_payload, write_size},
};
use crate::{
    SnapshotCompression, SnapshotDecodeLimits as Limits, SnapshotErrorReason, SnapshotWriteOptions,
    datatypes::crdts::{
        counter_crdt::CounterCrdt, map_crdt::MapCrdt, timed_node::TimedNode,
        variable_crdt::VariableCrdt,
    },
    types::{timestamp::Timestamp, uid::Cuid},
};
const WINDOW_LOG: u32 = 20;
fn none_options(codec: Codec) -> SnapshotWriteOptions {
    SnapshotWriteOptions {
        codec,
        compression: SnapshotCompression::None,
    }
}
const LIMITS: Limits = Limits {
    max_input_bytes: 4 * 1024 * 1024,
    max_raw_bytes: 4 * 1024 * 1024,
    max_map_entries: 4096,
};
fn codecs() -> Vec<Codec> {
    [1, 2].into_iter().filter_map(Codec::from_id).collect()
}
fn fixture(count: usize) -> MapCrdt {
    let mut state = MapCrdt::default();
    for i in 0..count {
        let cuid = if i % 2 == 0 {
            Cuid::new_nil()
        } else {
            Cuid::try_from("0000000000000001").unwrap()
        };
        state
            .replace(
                format!("k{i:04}").into(),
                Some(TimedNode {
                    ts: Timestamp::new(1, &cuid),
                    v: if i % 2 == 0 {
                        None
                    } else {
                        Some(Arc::from(&b"null"[..]))
                    },
                    approved_sseq_upper: None,
                }),
            )
            .unwrap();
    }
    state
}
#[test]
fn can_reject_unavailable_capabilities_and_invalid_envelopes() {
    assert_eq!(
        Codec::from_id(1).is_some(),
        cfg!(feature = "snapshot-bitcode")
    );
    assert_eq!(
        Codec::from_id(2).is_some(),
        cfg!(feature = "snapshot-postcard")
    );
    let empty =
        *b"QTSN\x01\x03\x01\x00\x0c\0\0\0\0\0\0\0\x0c\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0";
    for id in [1, 2] {
        let mut bytes = empty;
        bytes[6] = id;
        if Codec::from_id(id).is_none() {
            assert!(matches!(
                MapCrdt::decode_snapshot(&bytes, LIMITS),
                Err(SnapshotErrorReason::Unsupported)
            ));
            continue;
        }
        assert!(MapCrdt::decode_snapshot(&bytes, LIMITS).is_ok());
        for (offset, value) in [
            (0, b'X'),
            (4, 2),
            (5, 0),
            (5, 1),
            (6, 0),
            (7, 1),
            (8, 13),
            (16, 13),
        ] {
            let mut bad = bytes;
            bad[offset] = value;
            assert!(MapCrdt::decode_snapshot(&bad, LIMITS).is_err());
        }
        assert!(matches!(
            MapCrdt::decode_snapshot(
                &bytes,
                Limits {
                    max_input_bytes: 0,
                    ..LIMITS
                }
            ),
            Err(SnapshotErrorReason::LimitExceeded)
        ));
    }
}
#[test]
fn can_preserve_crdt_states_and_reject_incomplete_or_excess_input() {
    for codec in codecs() {
        for count in [0, 1, 31, 32, 33, 64, 65] {
            let state = fixture(count);
            let bytes = MapCrdt::encode_snapshot(&state, none_options(codec)).unwrap();
            assert!(MapCrdt::decode_snapshot(&bytes, LIMITS).unwrap() == state);
            for end in 0..bytes.len() {
                assert!(MapCrdt::decode_snapshot(&bytes[..end], LIMITS).is_err());
            }
            let exact = Limits {
                max_input_bytes: bytes.len(),
                max_raw_bytes: bytes.len() - HEADER_BYTES,
                max_map_entries: count,
            };
            assert!(MapCrdt::decode_snapshot(&bytes, exact).is_ok());
            assert!(matches!(
                MapCrdt::decode_snapshot(
                    &bytes,
                    Limits {
                        max_raw_bytes: exact.max_raw_bytes - 1,
                        ..exact
                    }
                ),
                Err(SnapshotErrorReason::LimitExceeded)
            ));
            if count > 0 {
                assert!(matches!(
                    MapCrdt::decode_snapshot(
                        &bytes,
                        Limits {
                            max_map_entries: count - 1,
                            ..exact
                        }
                    ),
                    Err(SnapshotErrorReason::LimitExceeded)
                ));
            }
            let mut extra = bytes.to_vec();
            extra.push(0);
            assert!(MapCrdt::decode_snapshot(&extra, LIMITS).is_err());
        }
        for value in [i64::MIN, 0, i64::MAX] {
            let bytes =
                CounterCrdt::encode_snapshot(&CounterCrdt { value }, none_options(codec)).unwrap();
            assert_eq!(
                CounterCrdt::decode_snapshot(&bytes, LIMITS).unwrap().value,
                value
            );
            for end in 0..bytes.len() {
                assert!(CounterCrdt::decode_snapshot(&bytes[..end], LIMITS).is_err());
            }
            let mut extra = bytes.to_vec();
            extra.push(0);
            assert!(CounterCrdt::decode_snapshot(&extra, LIMITS).is_err());
        }
        let timestamp = Timestamp::new(u64::MAX, &Cuid::new_nil());
        for value in [
            &b" { \"b\":1e400, \"a\":null } "[..],
            b"null",
            b"\"UTF8: \xe2\x98\x83\"",
        ] {
            let state = VariableCrdt::new(value, &timestamp);
            let bytes = VariableCrdt::encode_snapshot(&state, none_options(codec)).unwrap();
            assert!(VariableCrdt::decode_snapshot(&bytes, LIMITS).unwrap() == state);
            for end in 0..bytes.len() {
                assert!(VariableCrdt::decode_snapshot(&bytes[..end], LIMITS).is_err());
            }
            let mut extra = bytes.to_vec();
            extra.push(0);
            assert!(VariableCrdt::decode_snapshot(&extra, LIMITS).is_err());
        }
        let initial = VariableCrdt::default();
        assert!(
            VariableCrdt::decode_snapshot(
                &VariableCrdt::encode_snapshot(&initial, none_options(codec)).unwrap(),
                LIMITS
            )
            .unwrap()
                == initial
        );
    }
}
#[test]
fn can_match_frozen_map_wire_fixtures() {
    // Frozen with the separate framing prototype; only the empty header has an independent literal oracle.
    let fixtures: &[(u8, usize, &[u8])] = &[
        (1, 0, include_bytes!("fixtures/map-0-codec1-comp0.bin")),
        (1, 1, include_bytes!("fixtures/map-1-codec1-comp0.bin")),
        (1, 33, include_bytes!("fixtures/map-33-codec1-comp0.bin")),
        (2, 0, include_bytes!("fixtures/map-0-codec2-comp0.bin")),
        (2, 1, include_bytes!("fixtures/map-1-codec2-comp0.bin")),
        (2, 33, include_bytes!("fixtures/map-33-codec2-comp0.bin")),
    ];
    for &(id, count, bytes) in fixtures {
        if let Some(codec) = Codec::from_id(id) {
            let state = fixture(count);
            assert_eq!(
                &*MapCrdt::encode_snapshot(&state, none_options(codec)).unwrap(),
                bytes
            );
            assert!(MapCrdt::decode_snapshot(bytes, LIMITS).unwrap() == state);
        }
    }
}
fn map_bytes(
    codec: Codec,
    count: usize,
    cuid_table: &[[u8; 16]],
    blocks: &[SnapshotMapBlock<'_>],
) -> Vec<u8> {
    let mut raw = Vec::new();
    write_size(&mut raw, count).unwrap();
    raw.extend_from_slice(&(cuid_table.len() as u32).to_le_bytes());
    for cuid_bytes in cuid_table {
        raw.extend_from_slice(cuid_bytes);
    }
    for block in blocks {
        let bytes = encode_payload!(codec, block).unwrap();
        write_size(&mut raw, bytes.len()).unwrap();
        raw.extend(bytes);
    }
    encode_frame(3, none_options(codec), &raw, LIMITS, WINDOW_LOG)
        .unwrap()
        .into_vec()
}
#[test]
fn can_reject_codec_valid_domain_errors() {
    for codec in codecs() {
        let mut block: SnapshotMapBlock = std::array::from_fn(|_| padding());
        block[0] = SnapshotMapRow {
            key: "",
            lamport: 1,
            cuid_index: 0,
            value: Some("null"),
        };
        let nil = *b"0000000000000000";
        let valid = map_bytes(codec, 1, &[nil], &[block.clone()]);
        assert!(MapCrdt::decode_snapshot(&valid, LIMITS).is_ok());
        for bad_row in [
            SnapshotMapRow {
                lamport: 0,
                ..block[0].clone()
            },
            SnapshotMapRow {
                cuid_index: 1,
                ..block[0].clone()
            },
            SnapshotMapRow {
                value: Some("null null"),
                ..block[0].clone()
            },
        ] {
            let mut bad = block.clone();
            bad[0] = bad_row;
            assert!(matches!(
                MapCrdt::decode_snapshot(&map_bytes(codec, 1, &[nil], &[bad]), LIMITS),
                Err(SnapshotErrorReason::Invalid)
            ));
        }
        for row in [
            SnapshotMapRow {
                key: "bad",
                ..padding()
            },
            SnapshotMapRow {
                lamport: 1,
                ..padding()
            },
            SnapshotMapRow {
                cuid_index: 1,
                ..padding()
            },
            SnapshotMapRow {
                value: Some("null"),
                ..padding()
            },
        ] {
            let mut bad = block.clone();
            bad[1] = row;
            assert!(
                MapCrdt::decode_snapshot(&map_bytes(codec, 1, &[nil], &[bad]), LIMITS).is_err()
            );
        }
        for cuid_table in [vec![], vec![*b"!000000000000000"]] {
            assert!(
                MapCrdt::decode_snapshot(
                    &map_bytes(codec, 1, &cuid_table, &[block.clone()]),
                    LIMITS
                )
                .is_err()
            );
        }
        let mut two_rows = block.clone();
        two_rows[1] = SnapshotMapRow {
            key: "second",
            cuid_index: 1,
            ..two_rows[0].clone()
        };
        assert!(
            MapCrdt::decode_snapshot(
                &map_bytes(codec, 2, &[nil, *b"0000000000000001"], &[two_rows.clone()]),
                LIMITS,
            )
            .is_ok()
        );
        assert!(matches!(
            MapCrdt::decode_snapshot(&map_bytes(codec, 2, &[nil, nil], &[two_rows]), LIMITS),
            Err(SnapshotErrorReason::Invalid)
        ));
        let mut duplicate = block.clone();
        duplicate[1] = duplicate[0].clone();
        assert!(
            MapCrdt::decode_snapshot(&map_bytes(codec, 2, &[nil], &[duplicate]), LIMITS).is_err()
        );
        let mut full: SnapshotMapBlock = std::array::from_fn(|_| SnapshotMapRow {
            key: "z",
            lamport: 1,
            cuid_index: 0,
            value: None,
        });
        // Independently increasing keys within the first block, duplicate across blocks.
        let keys: Vec<_> = (0..32).map(|i| format!("k{i:04}")).collect();
        for (row, key) in full.iter_mut().zip(&keys) {
            row.key = key;
        }
        let mut last = block.clone();
        last[0].key = &keys[31];
        assert!(
            MapCrdt::decode_snapshot(&map_bytes(codec, 33, &[nil], &[full, last]), LIMITS).is_err()
        );
        for (lamport, cuid, value) in [
            (0, nil, " null "),
            (0, *b"0000000000000001", "null"),
            (1, nil, "{"),
            (1, *b"!000000000000000", "null"),
        ] {
            if let Ok(cuid) = Cuid::from_bytes(&cuid) {
                let state = VariableCrdt::new(value.as_bytes(), &Timestamp::new(lamport, &cuid));
                assert!(matches!(
                    state.encode_snapshot(none_options(codec)),
                    Err(SnapshotErrorReason::Invalid)
                ));
            }
            let dto = SnapshotVariable {
                lamport,
                cuid,
                value,
            };
            let bytes = encode_frame(
                2,
                none_options(codec),
                &encode_payload!(codec, &dto).unwrap(),
                LIMITS,
                WINDOW_LOG,
            )
            .unwrap();
            assert!(matches!(
                VariableCrdt::decode_snapshot(&bytes, LIMITS),
                Err(SnapshotErrorReason::Invalid)
            ));
        }
        // Declarations must be checked against available bytes before table allocation.
        let mut raw = u64::MAX.to_le_bytes().to_vec();
        raw.extend_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(
            MapCrdt::decode_snapshot(
                &encode_frame(3, none_options(codec), &raw, LIMITS, WINDOW_LOG).unwrap(),
                Limits {
                    max_map_entries: usize::MAX,
                    ..LIMITS
                }
            ),
            Err(SnapshotErrorReason::Invalid)
        ));
        let mut extra = valid;
        extra.push(0);
        let stored = extra.len() - HEADER_BYTES;
        extra[8..16].copy_from_slice(&(stored as u64).to_le_bytes());
        extra[16..24].copy_from_slice(&(stored as u64).to_le_bytes());
        assert!(MapCrdt::decode_snapshot(&extra, LIMITS).is_err());
    }
}

#[test]
fn can_preserve_map_json_and_reject_invalid_utf8_and_codec_trailing_bytes() {
    for codec in codecs() {
        let mut block: SnapshotMapBlock = std::array::from_fn(|_| padding());
        let value = " { \"b\":1e400, \"a\":null } ";
        block[0] = SnapshotMapRow {
            key: "",
            lamport: u64::MAX,
            cuid_index: 0,
            value: Some(value),
        };
        let bytes = map_bytes(codec, 1, &[*b"0000000000000000"], &[block.clone()]);
        let state = MapCrdt::decode_snapshot(&bytes, LIMITS).unwrap();
        let entry = state.entries.get("").unwrap();
        assert_eq!(entry.v.as_deref(), Some(value.as_bytes()));
        assert_eq!(entry.ts.lamport(), u64::MAX);
        assert_eq!(
            MapCrdt::encode_snapshot(&state, none_options(codec))
                .unwrap()
                .as_ref(),
            bytes
        );
        let dto = SnapshotVariable {
            lamport: 1,
            cuid: *b"0000000000000000",
            value: "\"UTF8MARKER\"",
        };
        let mut encoded = encode_payload!(codec, &dto).unwrap();
        let offset = encoded
            .windows(10)
            .position(|w| w == b"UTF8MARKER")
            .unwrap();
        encoded[offset] = 255;
        assert!(matches!(
            VariableCrdt::decode_snapshot(
                &encode_frame(2, none_options(codec), &encoded, LIMITS, WINDOW_LOG).unwrap(),
                LIMITS
            ),
            Err(SnapshotErrorReason::Invalid)
        ));
        let mut encoded = encode_payload!(codec, &dto).unwrap();
        encoded.push(0);
        assert!(
            VariableCrdt::decode_snapshot(
                &encode_frame(2, none_options(codec), &encoded, LIMITS, WINDOW_LOG).unwrap(),
                LIMITS
            )
            .is_err()
        );
        let mut encoded = encode_payload!(codec, &SnapshotCounter { value: 1 }).unwrap();
        encoded.push(0);
        assert!(
            CounterCrdt::decode_snapshot(
                &encode_frame(1, none_options(codec), &encoded, LIMITS, WINDOW_LOG).unwrap(),
                LIMITS
            )
            .is_err()
        );
        let mut bytes = bytes;
        let n = bytes.len() - HEADER_BYTES;
        // The block length begins after the envelope and the 28-byte Map prefix.
        let block_length_offset = HEADER_BYTES + 28;
        bytes[block_length_offset..block_length_offset + size_of::<u64>()]
            .copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(
            MapCrdt::decode_snapshot(
                &bytes,
                Limits {
                    max_raw_bytes: n,
                    ..LIMITS
                }
            )
            .is_err()
        );
    }
}

#[test]
fn can_report_owned_buffer_reservation_failure() {
    assert_eq!(
        buffer_with_capacity(usize::MAX),
        Err(SnapshotErrorReason::ResourceExhausted)
    );
}

#[test]
fn can_apply_writer_choices_independently_of_reader_limits() {
    for codec in codecs() {
        for compression in [0, 1].into_iter().filter_map(SnapshotCompression::from_id) {
            let options = SnapshotWriteOptions { codec, compression };
            let bytes = CounterCrdt::encode_snapshot(&CounterCrdt { value: 1 }, options).unwrap();
            assert_eq!(
                CounterCrdt::decode_snapshot(&bytes, LIMITS).unwrap().value,
                1
            );
            let (_, raw) = decode_frame(&bytes, 1, LIMITS, WINDOW_LOG).unwrap();
            assert_eq!(
                matches!(raw, Cow::Borrowed(_)),
                compression == SnapshotCompression::None
            );
            let exact = Limits {
                max_input_bytes: bytes.len(),
                max_raw_bytes: raw.len(),
                max_map_entries: 0,
            };
            assert!(CounterCrdt::decode_snapshot(&bytes, exact).is_ok());
            assert!(matches!(
                CounterCrdt::decode_snapshot(
                    &bytes,
                    Limits {
                        max_input_bytes: bytes.len() - 1,
                        ..exact
                    }
                ),
                Err(SnapshotErrorReason::LimitExceeded)
            ));
            assert!(matches!(
                CounterCrdt::decode_snapshot(
                    &bytes,
                    Limits {
                        max_raw_bytes: raw.len() - 1,
                        ..exact
                    }
                ),
                Err(SnapshotErrorReason::LimitExceeded)
            ));
            for end in 0..bytes.len() {
                assert!(CounterCrdt::decode_snapshot(&bytes[..end], LIMITS).is_err());
            }
            if compression.id() == 1 {
                for declared in [raw.len() - 1, raw.len() + 1] {
                    let mut damaged = bytes.clone();
                    damaged[16..24].copy_from_slice(&(declared as u64).to_le_bytes());
                    assert!(matches!(
                        CounterCrdt::decode_snapshot(&damaged, LIMITS),
                        Err(SnapshotErrorReason::Invalid)
                    ));
                }
                let mut damaged = bytes.to_vec();
                damaged.extend_from_slice(&bytes[HEADER_BYTES..]);
                let stored = damaged.len() - HEADER_BYTES;
                damaged[8..16].copy_from_slice(&(stored as u64).to_le_bytes());
                assert!(matches!(
                    CounterCrdt::decode_snapshot(&damaged, LIMITS),
                    Err(SnapshotErrorReason::Invalid)
                ));
                let mut damaged = bytes.clone();
                *damaged.last_mut().unwrap() ^= 1;
                assert!(matches!(
                    CounterCrdt::decode_snapshot(&damaged, LIMITS),
                    Err(SnapshotErrorReason::Invalid)
                ));
            }
            let variable = VariableCrdt::default();
            let bytes = VariableCrdt::encode_snapshot(&variable, options).unwrap();
            assert!(VariableCrdt::decode_snapshot(&bytes, LIMITS).unwrap() == variable);
            for count in [0, 1, 31, 32, 33, 64, 65] {
                let state = fixture(count);
                let bytes = MapCrdt::encode_snapshot(&state, options).unwrap();
                assert!(MapCrdt::decode_snapshot(&bytes, LIMITS).unwrap() == state);
                if count > 0 {
                    assert!(matches!(
                        MapCrdt::decode_snapshot(
                            &bytes,
                            Limits {
                                max_map_entries: count - 1,
                                ..LIMITS
                            }
                        ),
                        Err(SnapshotErrorReason::LimitExceeded)
                    ));
                }
            }
        }
    }
}

#[cfg(not(feature = "snapshot-zstd"))]
#[test]
fn can_reject_disabled_compression_before_decoding_payload() {
    for codec in codecs() {
        let mut bytes =
            CounterCrdt::encode_snapshot(&CounterCrdt { value: 1 }, none_options(codec)).unwrap();
        bytes[7] = 1;
        assert!(matches!(
            CounterCrdt::decode_snapshot(&bytes, LIMITS),
            Err(SnapshotErrorReason::Unsupported)
        ));
    }
}

#[cfg(all(feature = "snapshot-bitcode", feature = "snapshot-zstd"))]
#[test]
fn can_keep_fixed_default_writer_policy() {
    assert_eq!(
        SnapshotWriteOptions::default(),
        SnapshotWriteOptions {
            codec: Codec::Bitcode,
            compression: SnapshotCompression::Zstd1,
        }
    );
}

#[test]
fn can_encode_with_exact_snapshot_limits_including_tombstones() {
    use crate::{DatatypeError, datatypes::Crdt};
    let map = fixture(32);
    for codec in codecs() {
        let raw_len = map.encode_raw(codec, 32).unwrap().len();
        for compression in [0, 1].into_iter().filter_map(SnapshotCompression::from_id) {
            let options = SnapshotWriteOptions { codec, compression };
            let state = Crdt::Map(map.fork_for_batch());
            let frame = state.serialize(options).unwrap();
            let limits = Limits {
                max_input_bytes: frame.len(),
                max_raw_bytes: raw_len,
                max_map_entries: 32,
            };
            assert_eq!(state.serialize_with_limits(options, limits).unwrap(), frame);
            MapCrdt::decode_snapshot(&frame, limits).unwrap();
            for smaller in [
                Limits {
                    max_input_bytes: frame.len() - 1,
                    ..limits
                },
                Limits {
                    max_raw_bytes: raw_len - 1,
                    ..limits
                },
                Limits {
                    max_map_entries: 31,
                    ..limits
                },
            ] {
                assert!(matches!(
                    state.serialize_with_limits(options, smaller),
                    Err(DatatypeError::SnapshotEncode(
                        SnapshotErrorReason::LimitExceeded
                    ))
                ));
            }
        }
    }
}
