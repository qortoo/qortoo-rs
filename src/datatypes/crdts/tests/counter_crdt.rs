use tracing::{info, instrument};

use crate::{
    datatypes::{
        common::ReturnType,
        crdts::{RollbackAction, counter_crdt::CounterCrdt, snapshot::SnapshotCodec},
    },
    operations::Operation,
    types::{operation_context::OperationContext, uid::Cuid},
    utils::test_utils::snapshot_write_options,
};

#[test]
fn can_new_and_increase_counter() {
    let mut counter = CounterCrdt::default();
    counter.increase_by(1);
    counter.increase_by(-2);
    assert_eq!(counter.value(), -1);
}

#[test]
#[instrument]
fn can_wrap_counter_arithmetic_at_i64_boundaries() {
    let mut counter = CounterCrdt { value: i64::MAX };

    assert_eq!(counter.increase_by(1), i64::MIN);
    assert_eq!(counter.increase_by(-1), i64::MAX);
}

#[test]
fn can_serialize_and_deserialize_counter_crdt() {
    let mut counter = CounterCrdt::default();
    counter.increase_by(123);

    let serialized = counter.encode_snapshot(snapshot_write_options()).unwrap();
    info!("serialized counter: {serialized:?}");
    assert_eq!(&serialized[..4], b"QTSN");

    let deserialized = CounterCrdt::decode_snapshot(&serialized, Default::default()).unwrap();
    assert_eq!(deserialized.value(), counter.value());
}

#[test]
#[instrument]
fn can_reject_an_invalid_counter_snapshot_length() {
    let error = CounterCrdt::decode_snapshot(&[], Default::default()).unwrap_err();

    assert_eq!(error.to_string(), "invalid state or snapshot data");
}

#[test]
#[instrument]
fn can_return_and_apply_a_counter_rollback_action() {
    for (initial_value, delta, expected_value) in [(0, i64::MIN, i64::MIN), (i64::MAX, 1, i64::MIN)]
    {
        let mut counter = CounterCrdt {
            value: initial_value,
        };
        let mut operation = Operation::new_counter_increase(delta);
        operation.set_lamport(1);
        let context = OperationContext::try_new(&operation, &Cuid::default()).unwrap();
        let outcome = counter.execute_local_operation(&context).unwrap();
        let (return_value, action) = outcome.into_parts();
        let RollbackAction::Counter(action) = action else {
            panic!("expected a counter rollback action");
        };

        assert!(matches!(return_value, ReturnType::Counter(value) if value == expected_value));
        assert_eq!(counter.value(), expected_value);

        counter.apply_rollback_action(action).unwrap();
        assert_eq!(counter.value(), initial_value);
    }
}
