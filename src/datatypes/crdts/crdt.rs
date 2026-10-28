use derive_more::Display;

use super::{
    counter_crdt::CounterCrdt,
    execution::{LocalOperationOutcome, RollbackAction},
    snapshot_codec::SnapshotCodec,
    variable_crdt::VariableCrdt,
};
use crate::{
    DataType, DatatypeError, errors::datatypes::InternalReason,
    types::operation_context::OperationContext,
};

#[derive(Debug, Display)]
pub enum Crdt {
    Counter(CounterCrdt),
    Variable(VariableCrdt),
}

impl Crdt {
    /// Creates an isolated batch candidate in O(1), sharing immutable payloads.
    /// Never clone CRDT collections here; new datatypes must provide an O(1) fork.
    /// Batch writes must not defer a full collection copy to copy-on-write either.
    pub(crate) fn fork_for_batch(&self) -> Self {
        match self {
            Self::Counter(counter) => Self::Counter(counter.fork_for_batch()),
            Self::Variable(variable) => Self::Variable(variable.fork_for_batch()),
        }
    }

    pub fn new(r#type: DataType) -> Self {
        match r#type {
            DataType::Counter => Crdt::Counter(CounterCrdt::default()),
            DataType::Variable => Crdt::Variable(VariableCrdt::default()),
            DataType::Map => unreachable!("invalid type"),
        }
    }

    pub fn as_counter(&self) -> Option<&CounterCrdt> {
        match self {
            Self::Counter(counter) => Some(counter),
            Self::Variable(_) => None,
        }
    }

    pub fn as_variable(&self) -> Option<&VariableCrdt> {
        match self {
            Self::Counter(_) => None,
            Self::Variable(variable) => Some(variable),
        }
    }

    pub(crate) fn execute_local_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<LocalOperationOutcome, DatatypeError> {
        match self {
            Crdt::Counter(c) => c.execute_local_operation(context),
            Crdt::Variable(v) => v.execute_local_operation(context),
        }
    }

    pub(crate) fn execute_remote_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<(), DatatypeError> {
        match self {
            Crdt::Counter(c) => c.execute_remote_operation(context),
            Crdt::Variable(v) => v.execute_remote_operation(context),
        }
    }

    pub(crate) fn apply_rollback_action(
        &mut self,
        action: RollbackAction,
    ) -> Result<(), DatatypeError> {
        match (self, action) {
            (Crdt::Counter(c), RollbackAction::Counter(action)) => c.apply_rollback_action(action),
            (Crdt::Variable(v), RollbackAction::Variable(action)) => {
                v.apply_rollback_action(action)
            }
            _ => Err(InternalReason::ExecuteOperation(
                "rollback action does not match crdt".to_owned(),
            )
            .into_error()),
        }
    }

    pub fn serialize(&self) -> Box<[u8]> {
        match self {
            Self::Counter(c) => c.encode_snapshot(),
            Self::Variable(v) => v.encode_snapshot(),
        }
    }

    pub fn deserialize(&mut self, serialized: &[u8]) -> Result<(), DatatypeError> {
        match self {
            Self::Counter(c) => {
                *c = CounterCrdt::decode_snapshot(serialized)?;
                Ok(())
            }
            Self::Variable(v) => {
                *v = VariableCrdt::decode_snapshot(serialized)?;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests_crdt {
    use std::sync::Arc;

    use tracing::instrument;

    use crate::{
        DataType, DatatypeError,
        datatypes::{
            common::ReturnType,
            crdts::{Crdt, counter_crdt::CounterCrdt},
        },
        operations::Operation,
        types::{operation_context::OperationContext, uid::Cuid},
    };

    #[test]
    fn can_fork_a_variable_batch_without_copying_or_mutating_its_payload() {
        let mut source = Crdt::new(DataType::Variable);
        let cuid = Cuid::try_from("0000000000000001").unwrap();
        let mut operation =
            Operation::new_variable_set(br#""initial""#.to_vec().into_boxed_slice());
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
        let serialized = crdt1.serialize();
        crdt2.deserialize(&serialized).unwrap();

        let c = crdt2.as_counter().expect("expected a counter crdt");
        assert_eq!(c.value(), 100);

        // Invalid input returns Err; counter value must not change.
        assert!(matches!(
            crdt2.deserialize("{}".as_bytes()),
            Err(DatatypeError::Internal(_))
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

        let mut operation =
            Operation::new_variable_set(br#""updated""#.to_vec().into_boxed_slice());
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
        let serialized = source.serialize();
        let mut restored = Crdt::new(DataType::Variable);
        restored.deserialize(&serialized).unwrap();

        assert_eq!(restored.serialize(), serialized);
        let variable = restored.as_variable().expect("expected a variable crdt");
        assert_eq!(variable.value(), value);
        assert_eq!(variable.timestamp(), context.timestamp());

        let before_invalid_snapshot = restored.serialize();
        let error = restored.deserialize(&[1]).unwrap_err();
        assert!(matches!(error, DatatypeError::Internal(_)));
        assert_eq!(restored.serialize(), before_invalid_snapshot);
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

        assert!(
            error
                .to_string()
                .contains("rollback action does not match crdt")
        );
        let counter = counter.as_counter().expect("expected a counter crdt");
        assert_eq!(counter.value(), 0);
    }
}
