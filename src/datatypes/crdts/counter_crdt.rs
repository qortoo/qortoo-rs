use derive_more::Display;

use crate::{
    DatatypeError,
    datatypes::{
        common::ReturnType,
        crdts::{LocalOperationOutcome, RollbackAction},
    },
    errors::datatypes::InternalReason,
    operations::body::OperationBody,
    types::operation_context::OperationContext,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CounterRollbackAction {
    Increase { delta: i64 },
}

#[derive(Debug, Default, Clone, Display)]
pub struct CounterCrdt {
    value: i64,
}

impl CounterCrdt {
    pub fn increase_by(&mut self, delta: i64) -> i64 {
        self.value = self.value.wrapping_add(delta);
        self.value
    }

    pub fn value(&self) -> i64 {
        self.value
    }

    pub(crate) fn execute_local_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<LocalOperationOutcome, DatatypeError> {
        let op = context.operation();
        match op.body {
            OperationBody::CounterIncrease(ref body) => {
                let rollback_action = RollbackAction::Counter(CounterRollbackAction::Increase {
                    delta: body.delta.wrapping_neg(),
                });
                let ret = self.increase_by(body.delta);
                Ok(LocalOperationOutcome::new(
                    ReturnType::Counter(ret),
                    rollback_action,
                ))
            }
            #[allow(unreachable_patterns)]
            _ => Err(InternalReason::ExecuteOperation(
                "counter cannot execute this local operation".to_owned(),
            )
            .into_error()),
        }
    }

    pub(crate) fn execute_remote_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<(), DatatypeError> {
        match context.operation().body {
            OperationBody::CounterIncrease(ref body) => {
                self.increase_by(body.delta);
                Ok(())
            }
            #[allow(unreachable_patterns)]
            _ => Err(InternalReason::ExecuteOperation(
                "counter cannot execute this remote operation".to_owned(),
            )
            .into_error()),
        }
    }

    pub(crate) fn apply_rollback_action(
        &mut self,
        action: CounterRollbackAction,
    ) -> Result<(), DatatypeError> {
        match action {
            CounterRollbackAction::Increase { delta } => {
                self.increase_by(delta);
                Ok(())
            }
        }
    }

    #[inline]
    pub fn to_bytes(&self) -> [u8; 8] {
        self.value.to_le_bytes()
    }

    #[inline]
    pub fn from_bytes(bytes: &[u8; 8]) -> Self {
        Self {
            value: i64::from_le_bytes(*bytes),
        }
    }
}

#[cfg(test)]
mod tests_counter_crdt {
    use tracing::{info, instrument};

    use crate::{
        datatypes::{
            common::ReturnType,
            crdts::{RollbackAction, counter_crdt::CounterCrdt},
        },
        operations::Operation,
        types::{operation_context::OperationContext, uid::Cuid},
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
        let mut counter = CounterCrdt::from_bytes(&i64::MAX.to_le_bytes());

        assert_eq!(counter.increase_by(1), i64::MIN);
        assert_eq!(counter.increase_by(-1), i64::MAX);
    }

    #[test]
    fn can_serialize_and_deserialize_counter_crdt() {
        let mut counter = CounterCrdt::default();
        counter.increase_by(123);

        let serialized = counter.to_bytes();
        info!("serialized counter: {serialized:?}");
        assert_eq!(serialized, 123_i64.to_le_bytes());

        let deserialized: CounterCrdt = CounterCrdt::from_bytes(&serialized);
        assert_eq!(deserialized.value(), counter.value());
    }

    #[test]
    #[instrument]
    fn can_return_and_apply_a_counter_rollback_action() {
        for (initial_value, delta, expected_value) in
            [(0, i64::MIN, i64::MIN), (i64::MAX, 1, i64::MIN)]
        {
            let mut counter = CounterCrdt::from_bytes(&initial_value.to_le_bytes());
            let operation = Operation::new_counter_increase(delta);
            let context = OperationContext::new(&operation, &Cuid::default());
            let outcome = counter.execute_local_operation(&context).unwrap();
            let (return_value, action) = outcome.into_parts();
            let RollbackAction::Counter(action) = action else {
                panic!("counter returned a non-counter rollback action");
            };

            assert!(matches!(return_value, ReturnType::Counter(value) if value == expected_value));
            assert_eq!(counter.value(), expected_value);

            counter.apply_rollback_action(action).unwrap();
            assert_eq!(counter.value(), initial_value);
        }
    }
}
