use std::sync::Arc;

use crate::{
    DatatypeError, SnapshotErrorReason,
    datatypes::{
        common::ReturnType,
        crdts::{
            Crdt, LocalOperationOutcome, RollbackAction, map_crdt::MapCrdt, snapshot::SnapshotCodec,
        },
    },
    operations::Operation,
    types::{operation_context::OperationContext, uid::Cuid},
    utils::test_utils::snapshot_write_options,
};

fn operation(key: &str, value: Option<&[u8]>, lamport: u64) -> Operation {
    let mut op = match value {
        Some(value) => Operation::new_map_put(key.into(), value.into()),
        None => Operation::new_map_remove(key.into()),
    };
    op.set_lamport(lamport);
    op
}

fn remote(map: &mut MapCrdt, op: &Operation, writer: &str) -> Result<(), DatatypeError> {
    map.execute_remote_operation(&OperationContext::try_new(
        op,
        &Cuid::try_from(writer).unwrap(),
    )?)
}

fn local(
    map: &mut MapCrdt,
    key: &str,
    value: Option<&[u8]>,
    lamport: u64,
) -> LocalOperationOutcome {
    let op = operation(key, value, lamport);
    map.execute_local_operation(&OperationContext::try_new(&op, &Cuid::new_nil()).unwrap())
        .unwrap()
}

fn previous(outcome: &LocalOperationOutcome) -> Option<&[u8]> {
    let (LocalOperationOutcome::Applied {
        return_value: ReturnType::Map(value),
        ..
    }
    | LocalOperationOutcome::NoOp {
        return_value: ReturnType::Map(value),
    }) = outcome
    else {
        panic!("expected a map result")
    };
    value.as_deref()
}

#[test]
fn can_return_previous_values_and_distinguish_null_from_missing() {
    let mut map = MapCrdt::default();
    assert!(map.is_empty());
    assert_eq!(previous(&local(&mut map, "k", Some(b"null"), 1)), None);
    assert_eq!(map.shared_value("k").as_deref(), Some(b"null".as_slice()));
    assert_eq!(map.len(), 1);
    assert_eq!(
        previous(&local(&mut map, "k", Some(b"9007199254740993"), 2)),
        Some(b"null".as_slice())
    );
    assert_eq!(
        previous(&local(&mut map, "k", None, 3)),
        Some(b"9007199254740993".as_slice())
    );
    assert_eq!(map.shared_value("k"), None);
    assert_eq!(
        (map.len(), map.tombstone_count, map.tombstone_bytes),
        (0, 1, 43)
    );
    let before = map.fork_for_batch();
    assert!(matches!(
        local(&mut map, "k", None, 1),
        LocalOperationOutcome::NoOp { .. }
    ));
    assert!(matches!(
        local(&mut map, "absent", None, 4),
        LocalOperationOutcome::NoOp { .. }
    ));
    assert_eq!(map, before);
    local(&mut map, "k", Some(b"true"), 4);
    assert_eq!(
        (map.len(), map.tombstone_count, map.tombstone_bytes),
        (1, 0, 0)
    );
}

#[test]
fn can_put_remove_and_snapshot_an_empty_key_locally_and_remotely() {
    let mut local_map = MapCrdt::default();
    let mut remote_map = MapCrdt::default();
    for (index, value) in [
        Some(b"1".as_slice()),
        Some(b"null".as_slice()),
        None,
        Some(b"true".as_slice()),
    ]
    .into_iter()
    .enumerate()
    {
        let lamport = index as u64 + 1;
        local(&mut local_map, "", value, lamport);
        let op = operation("", value, lamport);
        let context = OperationContext::try_new(&op, &Cuid::new_nil()).unwrap();
        remote_map.execute_remote_operation(&context).unwrap();
        assert_eq!(local_map, remote_map);
        assert_eq!(local_map.shared_value("").as_deref(), value);
        assert_eq!(
            MapCrdt::decode_snapshot(
                &local_map.encode_snapshot(snapshot_write_options()).unwrap(),
                Default::default()
            )
            .unwrap(),
            local_map
        );
        if value.is_none() {
            assert_eq!(local_map.tombstone_bytes, 42);
        }
    }
}

#[test]
fn can_restore_absent_live_and_approved_tombstone_entries_in_reverse_order() {
    let mut map = MapCrdt::default();
    local(&mut map, "한 글/키\0", Some(b"1"), 1);
    local(&mut map, "한 글/키\0", None, 2);
    map.entries
        .get_mut("한 글/키\0")
        .unwrap()
        .approved_sseq_upper = Some(9);
    let original = map.fork_for_batch();
    let mut actions = Vec::new();
    for (key, value, timestamp) in [
        ("한 글/키\0", Some(b"null".as_slice()), 3),
        ("한 글/키\0", Some(b"[true]".as_slice()), 4),
        ("한 글/키\0", None, 5),
        ("new", Some(b"{}".as_slice()), 6),
        ("new", None, 7),
    ] {
        let (_, action) = local(&mut map, key, value, timestamp).into_parts();
        assert!(!format!("{action:?}").contains("한 글"));
        actions.push(action);
    }
    for action in actions.into_iter().rev() {
        let RollbackAction::Map(action) = action else {
            panic!("expected a map rollback")
        };
        map.apply_rollback_action(action).unwrap();
    }
    assert_eq!(map, original);
    assert_eq!(
        map.entries.get("한 글/키\0").unwrap().approved_sseq_upper,
        Some(9)
    );
}

fn permutations(items: &mut [usize], index: usize, action: &mut impl FnMut(&[usize])) {
    if index == items.len() {
        action(items);
        return;
    }
    for next in index..items.len() {
        items.swap(index, next);
        permutations(items, index + 1, action);
        items.swap(index, next);
    }
}

#[test]
fn can_converge_under_every_delivery_order_and_duplicate_replay() {
    // Includes remove-before-put, reinsertion, same-Lamport CUID ties and independent keys.
    let events = [
        (operation("k", Some(b"1"), 1), "0000000000000001"),
        (operation("k", None, 2), "0000000000000001"),
        (operation("k", Some(b"null"), 3), "0000000000000001"),
        (operation("k", None, 3), "0000000000000002"),
        (
            operation("other", Some(b"{ \"b\": 2, \"a\": 1 }"), 1),
            "0000000000000001",
        ),
    ];
    let mut expected = MapCrdt::default();
    for (operation, writer) in &events {
        remote(&mut expected, operation, writer).unwrap();
    }
    let mut count = 0;
    permutations(&mut [0, 1, 2, 3, 4], 0, &mut |order| {
        let mut map = MapCrdt::default();
        for index in order.iter().chain(order.iter().rev()) {
            let (op, writer) = &events[*index];
            remote(&mut map, op, writer).unwrap();
        }
        assert_eq!(map, expected, "{order:?}");
        assert_eq!(
            map.encode_snapshot(snapshot_write_options()).unwrap(),
            expected.encode_snapshot(snapshot_write_options()).unwrap()
        );
        count += 1;
    });
    assert_eq!(count, 120);
    assert_eq!((expected.len(), expected.tombstone_count), (1, 1));
}

#[test]
fn can_advance_tombstones_without_decrementing_live_count_again() {
    let mut map = MapCrdt::default();
    remote(&mut map, &operation("k", None, 2), "0000000000000001").unwrap();
    assert_eq!(
        (map.len(), map.tombstone_count, map.tombstone_bytes),
        (0, 1, 43)
    );
    map.entries.get_mut("k").unwrap().approved_sseq_upper = Some(10);
    remote(&mut map, &operation("k", None, 3), "0000000000000001").unwrap();
    assert_eq!(
        (map.len(), map.tombstone_count, map.tombstone_bytes),
        (0, 1, 43)
    );
    assert_eq!(map.entries.get("k").unwrap().approved_sseq_upper, None);
    remote(&mut map, &operation("k", Some(b"1"), 2), "0000000000000001").unwrap();
    assert_eq!(map.shared_value("k"), None);
    remote(&mut map, &operation("k", Some(b"2"), 4), "0000000000000001").unwrap();
    assert_eq!(
        (map.len(), map.tombstone_count, map.tombstone_bytes),
        (1, 0, 0)
    );
}

#[test]
fn can_reject_ties_with_different_raw_bytes_or_operation_kinds() {
    for (winning, conflict) in [
        (Some(b"1".as_slice()), Some(b"2".as_slice())),
        (Some(b"1".as_slice()), Some(b" 1 ".as_slice())),
        (
            Some(br#"{"a":1,"b":2}"#.as_slice()),
            Some(br#"{"b":2,"a":1}"#.as_slice()),
        ),
        (Some(b"null".as_slice()), None),
        (None, Some(b"null".as_slice())),
    ] {
        let mut map = MapCrdt::default();
        remote(
            &mut map,
            &operation("secret-key", winning, 3),
            "0000000000000001",
        )
        .unwrap();
        let before = map.fork_for_batch();
        let error = remote(
            &mut map,
            &operation("secret-key", conflict, 3),
            "0000000000000001",
        )
        .unwrap_err();
        assert!(matches!(error, DatatypeError::Internal(_)));
        assert!(
            error
                .to_string()
                .contains("received different values for timestamp")
        );
        assert!(!format!("{error:?}").contains("secret"));
        assert_eq!(map, before);
    }
}

#[test]
fn can_apply_payload_bytes_without_validating_json() {
    let mut map = MapCrdt::default();
    local(&mut map, "key", Some(b"opaque-local"), 1);
    assert_eq!(
        map.shared_value("key").as_deref(),
        Some(b"opaque-local".as_slice())
    );
    remote(
        &mut map,
        &operation("key", Some(b"opaque-remote"), 2),
        "0000000000000001",
    )
    .unwrap();
    assert_eq!(
        map.shared_value("key").as_deref(),
        Some(b"opaque-remote".as_slice())
    );
}

#[test]
fn can_reject_local_operations_without_advancing_the_winning_timestamp() {
    let mut map = MapCrdt::default();
    local(&mut map, "key", Some(b"0"), 5);
    let before = map.fork_for_batch();
    for lamport in [1, 5] {
        for value in [Some(b"1".as_slice()), None] {
            let op = operation("key", value, lamport);
            let context = OperationContext::try_new(&op, &Cuid::new_nil()).unwrap();
            assert!(map.execute_local_operation(&context).is_err());
            assert_eq!(map, before);
        }
    }
}

#[test]
fn can_fork_shared_roots_and_isolate_payloads_metadata_and_rollback() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MapCrdt>();
    let mut map = MapCrdt::default();
    for index in 0..1024 {
        local(&mut map, &format!("k{index}"), Some(b"null"), index + 1);
    }
    local(&mut map, "deleted", Some(b"true"), 1025);
    local(&mut map, "deleted", None, 1026);
    map.entries.get_mut("deleted").unwrap().approved_sseq_upper = Some(42);
    let mut forked = map.fork_for_batch();
    assert!(map.entries.ptr_eq(&forked.entries));
    assert!(Arc::ptr_eq(
        &map.shared_value("k0").unwrap(),
        &forked.shared_value("k0").unwrap()
    ));
    forked
        .entries
        .get_mut("deleted")
        .unwrap()
        .approved_sseq_upper = Some(50);
    let (_, rollback) = local(&mut forked, "k0", None, 1027).into_parts();
    assert_eq!(map.len(), 1024);
    assert_eq!(forked.len(), 1023);
    assert_eq!(
        map.entries.get("deleted").unwrap().approved_sseq_upper,
        Some(42)
    );
    assert!(map.shared_value("k0").is_some());
    let RollbackAction::Map(rollback) = rollback else {
        panic!("expected map rollback")
    };
    forked.apply_rollback_action(rollback).unwrap();
    assert_eq!(map.live_count, forked.live_count);
    assert_eq!(map.tombstone_bytes, forked.tombstone_bytes);
    assert_eq!(map.logical_state_bytes(), forked.logical_state_bytes());
}

#[test]
fn can_round_trip_deterministic_snapshots_without_serializing_approvals() {
    let empty = MapCrdt::default();
    assert_eq!(
        MapCrdt::decode_snapshot(
            &empty.encode_snapshot(snapshot_write_options()).unwrap(),
            Default::default()
        )
        .unwrap(),
        empty
    );
    let mut map = MapCrdt::default();
    local(&mut map, "z", Some(br#"{ "b": 2, "a": 1 }"#), 1);
    local(&mut map, "a", Some(b"null"), 2);
    local(&mut map, "한 글/키\0", Some(b"9007199254740993"), 3);
    local(&mut map, "", Some(b"false"), 4);
    remote(&mut map, &operation("gone", None, 4), "0000000000000000").unwrap();
    let bytes = map.encode_snapshot(snapshot_write_options()).unwrap();
    map.entries.get_mut("gone").unwrap().approved_sseq_upper = Some(99);
    assert_eq!(
        map.encode_snapshot(snapshot_write_options()).unwrap(),
        bytes
    );
    let decoded = MapCrdt::decode_snapshot(&bytes, Default::default()).unwrap();
    let recounted: u64 = decoded
        .entries
        .iter()
        .map(|(key, entry)| {
            42 + key.len() as u64 + entry.v.as_deref().unwrap_or_default().len() as u64
        })
        .sum();
    assert_eq!(map.logical_state_bytes(), recounted);
    assert_eq!(decoded.logical_state_bytes(), recounted);
    assert_eq!(
        decoded.entries.get("gone").unwrap().approved_sseq_upper,
        None
    );
    assert_eq!(
        (
            decoded.len(),
            decoded.tombstone_count,
            decoded.tombstone_bytes
        ),
        (4, 1, 46)
    );
    assert_eq!(
        decoded.encode_snapshot(snapshot_write_options()).unwrap(),
        bytes
    );
    assert_eq!(decoded.shared_value("z"), map.shared_value("z"));
    let mut restored = decoded;
    remote(
        &mut restored,
        &operation("gone", Some(b"true"), 3),
        "0000000000000000",
    )
    .unwrap();
    assert!(restored.shared_value("gone").is_none());
    remote(
        &mut restored,
        &operation("gone", Some(b"true"), 5),
        "0000000000000000",
    )
    .unwrap();
    assert!(restored.shared_value("gone").is_some());
}

#[test]
fn can_reject_malformed_snapshots_without_replacing_the_crdt() {
    let mut crdt = Crdt::new(crate::DataType::Map);
    let op = operation("kept", Some(b"true"), 1);
    crdt.execute_local_operation(&OperationContext::try_new(&op, &Cuid::new_nil()).unwrap())
        .unwrap();
    let before = crdt.serialize(snapshot_write_options()).unwrap();
    let mut source = MapCrdt::default();
    local(&mut source, "a", Some(b"null"), 1);
    let valid = source.encode_snapshot(snapshot_write_options()).unwrap();
    let mut cases = (0..valid.len())
        .map(|end| valid[..end].to_vec())
        .collect::<Vec<_>>();
    let mut trailing = valid.to_vec();
    trailing.push(0);
    cases.push(trailing);
    for (index, bytes) in cases.iter().enumerate() {
        assert!(
            matches!(
                crdt.deserialize(bytes, Default::default()),
                Err(DatatypeError::SnapshotDecode(SnapshotErrorReason::Invalid))
            ),
            "case {index}"
        );
        assert_eq!(crdt.serialize(snapshot_write_options()).unwrap(), before);
    }
    crdt.deserialize(&valid, Default::default()).unwrap();
    assert_eq!(
        crdt.as_map().unwrap().shared_value("a").as_deref(),
        Some(b"null".as_slice())
    );
}
