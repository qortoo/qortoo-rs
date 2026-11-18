use std::sync::Arc;

use tracing::instrument;

use crate::{
    DataType, DatatypeError, SnapshotErrorReason,
    datatypes::{
        common::ReturnType,
        crdts::{Crdt, counter_crdt::CounterCrdt},
    },
    operations::Operation,
    types::{operation_context::OperationContext, uid::Cuid},
    utils::test_utils::snapshot_write_options,
};

#[test]
fn can_report_unsupported_operations_with_a_shared_message() {
    let cuid = Cuid::new_nil();
    for (datatype, mut operation) in [
        (
            DataType::Counter,
            Operation::new_variable_set(b"null".to_vec().into_boxed_slice()),
        ),
        (DataType::Variable, Operation::new_counter_increase(1)),
        (DataType::Map, Operation::new_counter_increase(1)),
    ] {
        operation.set_lamport(1);
        let context = OperationContext::try_new(&operation, &cuid).unwrap();
        let mut crdt = Crdt::new(datatype);
        let before = crdt.serialize(snapshot_write_options()).unwrap();
        for error in [
            crdt.execute_local_operation(&context).unwrap_err(),
            crdt.execute_remote_operation(&context).unwrap_err(),
        ] {
            assert_eq!(
                error.to_string(),
                "[E202] internal error: execute operation: operation is not supported by this datatype"
            );
        }
        assert_eq!(crdt.serialize(snapshot_write_options()).unwrap(), before);
    }
}

#[test]
fn can_fork_a_variable_batch_without_copying_or_mutating_its_payload() {
    let mut source = Crdt::new(DataType::Variable);
    let cuid = Cuid::try_from("0000000000000001").unwrap();
    let mut operation = Operation::new_variable_set(br#""initial""#.to_vec().into_boxed_slice());
    operation.set_lamport(1);
    source
        .execute_remote_operation(&OperationContext::try_new(&operation, &cuid).unwrap())
        .unwrap();
    let mut candidate = source.fork_for_batch();
    let original = source.as_variable().unwrap();
    let forked = candidate.as_variable().unwrap();
    assert!(Arc::ptr_eq(
        &original.shared_value(),
        &forked.shared_value()
    ));
    assert_eq!(original.timestamp(), forked.timestamp());

    let mut update = Operation::new_variable_set(br#""changed""#.to_vec().into_boxed_slice());
    update.set_lamport(2);
    candidate
        .execute_remote_operation(&OperationContext::try_new(&update, &cuid).unwrap())
        .unwrap();
    assert_eq!(source.as_variable().unwrap().value(), br#""initial""#);
    assert_eq!(source.as_variable().unwrap().timestamp().lamport(), 1);
    assert_eq!(candidate.as_variable().unwrap().value(), br#""changed""#);
}

#[test]
fn can_serialize_and_deserialize() {
    let mut counter = CounterCrdt::default();
    counter.increase_by(100);
    let crdt1 = Crdt::Counter(counter);

    let mut crdt2 = Crdt::new(DataType::Counter);
    let serialized = crdt1.serialize(snapshot_write_options()).unwrap();
    crdt2.deserialize(&serialized, Default::default()).unwrap();

    let c = crdt2.as_counter().expect("expected a counter crdt");
    assert_eq!(c.value(), 100);

    // Invalid input returns Err; counter value must not change.
    assert!(matches!(
        crdt2.deserialize("{}".as_bytes(), Default::default()),
        Err(DatatypeError::SnapshotDecode(SnapshotErrorReason::Invalid))
    ));
    let c = crdt2.as_counter().expect("expected a counter crdt");
    assert_eq!(c.value(), 100);
}

#[test]
#[instrument]
fn can_access_only_matching_crdt_variants() {
    let counter = Crdt::new(DataType::Counter);
    assert!(counter.as_counter().is_some());
    assert!(counter.as_variable().is_none());

    let variable = Crdt::new(DataType::Variable);
    assert!(variable.as_counter().is_none());
    assert!(variable.as_variable().is_some());
}

#[test]
#[instrument]
fn can_dispatch_local_variable_execution_and_rollback() {
    let mut crdt = Crdt::new(DataType::Variable);
    let variable = crdt.as_variable().expect("expected a variable crdt");
    assert_eq!(variable.value(), b"null");
    assert!(variable.timestamp().is_initial());

    let mut operation = Operation::new_variable_set(br#""updated""#.to_vec().into_boxed_slice());
    operation.set_lamport(1);
    let cuid = Cuid::try_from("0000000000000001").unwrap();
    let context = OperationContext::try_new(&operation, &cuid).unwrap();

    let outcome = crdt.execute_local_operation(&context).unwrap();
    let (return_value, rollback_action) = outcome.into_parts();
    let ReturnType::Variable(previous_value) = return_value else {
        panic!("expected a variable return value");
    };
    assert_eq!(previous_value.as_ref(), b"null");
    let variable = crdt.as_variable().expect("expected a variable crdt");
    assert_eq!(variable.value(), br#""updated""#);
    assert_eq!(variable.timestamp(), context.timestamp());

    crdt.apply_rollback_action(rollback_action).unwrap();
    let variable = crdt.as_variable().expect("expected a variable crdt");
    assert_eq!(variable.value(), b"null");
    assert!(variable.timestamp().is_initial());
}

#[test]
#[instrument]
fn can_dispatch_remote_variable_execution_and_snapshot_round_trip() {
    let mut source = Crdt::new(DataType::Variable);
    let value = br#"{"name":"qortoo"}"#;
    let mut operation = Operation::new_variable_set(value.to_vec().into_boxed_slice());
    operation.set_lamport(7);
    let cuid = Cuid::try_from("0000000000000001").unwrap();
    let context = OperationContext::try_new(&operation, &cuid).unwrap();

    source.execute_remote_operation(&context).unwrap();
    let display = source.to_string();
    assert_eq!(
        display,
        format!("(size:{}, timestamp:{})", value.len(), context.timestamp())
    );
    assert!(!display.contains("qortoo"));
    let serialized = source.serialize(snapshot_write_options()).unwrap();
    let mut restored = Crdt::new(DataType::Variable);
    restored
        .deserialize(&serialized, Default::default())
        .unwrap();

    assert_eq!(
        restored.serialize(snapshot_write_options()).unwrap(),
        serialized
    );
    let variable = restored.as_variable().expect("expected a variable crdt");
    assert_eq!(variable.value(), value);
    assert_eq!(variable.timestamp(), context.timestamp());

    let before_invalid_snapshot = restored.serialize(snapshot_write_options()).unwrap();
    let error = restored.deserialize(&[1], Default::default()).unwrap_err();
    assert!(matches!(
        error,
        DatatypeError::SnapshotDecode(SnapshotErrorReason::Invalid)
    ));
    assert_eq!(
        restored.serialize(snapshot_write_options()).unwrap(),
        before_invalid_snapshot
    );
}

#[test]
#[instrument]
fn can_reject_a_rollback_action_for_a_different_crdt() {
    let mut variable = Crdt::new(DataType::Variable);
    let mut operation = Operation::new_variable_set(b"true".to_vec().into_boxed_slice());
    operation.set_lamport(1);
    let context = OperationContext::try_new(&operation, &Cuid::default()).unwrap();
    let outcome = variable.execute_local_operation(&context).unwrap();
    let (_, rollback_action) = outcome.into_parts();
    let mut counter = Crdt::new(DataType::Counter);

    let error = counter.apply_rollback_action(rollback_action).unwrap_err();

    assert_eq!(
        error.to_string(),
        "[E202] internal error: execute operation: rollback action does not match the CRDT type"
    );
    let counter = counter.as_counter().expect("expected a counter crdt");
    assert_eq!(counter.value(), 0);
}

#[test]
fn can_distinguish_snapshot_encoding_from_decoding_failures() {
    use crate::{datatypes::crdts::variable_crdt::VariableCrdt, types::timestamp::Timestamp};

    let invalid = Crdt::Variable(VariableCrdt::new(b"{", &Timestamp::initial()));
    assert!(matches!(
        invalid.serialize(snapshot_write_options()),
        Err(DatatypeError::SnapshotEncode(SnapshotErrorReason::Invalid))
    ));

    for datatype in [DataType::Counter, DataType::Variable, DataType::Map] {
        let mut crdt = Crdt::new(datatype);
        let before = crdt.serialize(snapshot_write_options()).unwrap();
        assert!(matches!(
            crdt.deserialize(b"", Default::default()),
            Err(DatatypeError::SnapshotDecode(SnapshotErrorReason::Invalid))
        ));
        assert!(matches!(
            crdt.deserialize(
                &before,
                crate::SnapshotDecodeLimits {
                    max_input_bytes: 0,
                    ..Default::default()
                }
            ),
            Err(DatatypeError::SnapshotDecode(
                SnapshotErrorReason::LimitExceeded
            ))
        ));
        let mut unsupported = before.to_vec();
        unsupported[4] = 255;
        assert!(matches!(
            crdt.deserialize(&unsupported, Default::default()),
            Err(DatatypeError::SnapshotDecode(
                SnapshotErrorReason::Unsupported
            ))
        ));
        assert_eq!(crdt.serialize(snapshot_write_options()).unwrap(), before);
    }
}
