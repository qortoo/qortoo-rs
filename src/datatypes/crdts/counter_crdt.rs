use derive_more::Display;

use crate::{
    DatatypeError,
    datatypes::{
        common::ReturnType,
        crdts::{LocalOperationOutcome, RollbackAction},
    },
    types::operation_context::OperationContext,
};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum CounterRollbackAction {
    Increase { delta: i64 },
}

#[derive(Debug, Default, Display)]
pub struct CounterCrdt {
    pub(super) value: i64,
}

impl CounterCrdt {
    pub(crate) fn fork_for_batch(&self) -> Self {
        Self { value: self.value }
    }

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
        let delta = context.counter_input()?;
        let rollback_action = RollbackAction::Counter(CounterRollbackAction::Increase {
            delta: delta.wrapping_neg(),
        });
        let ret = self.increase_by(delta);
        Ok(LocalOperationOutcome::new(
            ReturnType::Counter(ret),
            rollback_action,
        ))
    }

    pub(crate) fn execute_remote_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<(), DatatypeError> {
        self.increase_by(context.counter_input()?);
        Ok(())
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
}
