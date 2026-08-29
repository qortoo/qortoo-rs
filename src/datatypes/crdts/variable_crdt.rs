#![allow(
    dead_code,
    reason = "Variable CRDT is connected to the top-level Crdt enum in the next slice"
)]

use std::{
    cmp::Ordering,
    fmt::{Debug, Formatter},
    sync::Arc,
};

use crate::{
    DatatypeError,
    datatypes::{
        common::ReturnType,
        crdts::{LocalOperationOutcome, RollbackAction},
    },
    errors::datatypes::InternalReason,
    operations::body::OperationBody,
    types::{operation_context::OperationContext, timestamp::Timestamp},
};

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct VariableState {
    value: Arc<[u8]>,
    timestamp: Timestamp,
}

impl VariableState {
    fn new(value: &[u8], timestamp: &Timestamp) -> Self {
        Self {
            value: value.into(),
            timestamp: timestamp.clone(),
        }
    }

    pub(crate) fn value(&self) -> &[u8] {
        &self.value
    }

    pub(crate) fn timestamp(&self) -> &Timestamp {
        &self.timestamp
    }
}

impl Debug for VariableState {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VariableState")
            .field("value_size", &self.value.len())
            .field("timestamp", &self.timestamp)
            .finish()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VariableSetOutcome {
    Applied { previous: Option<VariableState> },
    Unchanged,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VariableRollbackAction {
    Restore { previous: Option<VariableState> },
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct VariableCrdt {
    winning: Option<VariableState>,
}

impl VariableCrdt {
    pub(crate) fn value(&self) -> Option<&[u8]> {
        self.winning.as_ref().map(VariableState::value)
    }

    pub(crate) fn timestamp(&self) -> Option<&Timestamp> {
        self.winning.as_ref().map(VariableState::timestamp)
    }

    pub(crate) fn apply_set(
        &mut self,
        value: &[u8],
        timestamp: &Timestamp,
    ) -> Result<VariableSetOutcome, DatatypeError> {
        let Some(current) = &self.winning else {
            return Ok(self.replace(value, timestamp));
        };

        match timestamp.cmp(current.timestamp()) {
            Ordering::Greater => Ok(self.replace(value, timestamp)),
            Ordering::Less => Ok(VariableSetOutcome::Unchanged),
            Ordering::Equal if value == current.value() => Ok(VariableSetOutcome::Unchanged),
            Ordering::Equal => Err(InternalReason::ExecuteOperation(format!(
                "variable received different values for timestamp {timestamp}"
            ))
            .into_error()),
        }
    }

    pub(crate) fn execute_local_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<LocalOperationOutcome, DatatypeError> {
        let OperationBody::VariableSet(body) = &context.operation().body else {
            return Err(InternalReason::ExecuteOperation(
                "variable cannot execute this local operation".to_owned(),
            )
            .into_error());
        };

        let VariableSetOutcome::Applied { previous } =
            self.apply_set(&body.value, context.timestamp())?
        else {
            return Err(InternalReason::ExecuteOperation(
                "local variable set did not advance the winning timestamp".to_owned(),
            )
            .into_error());
        };
        let previous_value = previous
            .as_ref()
            .map(|previous| Arc::clone(&previous.value));

        Ok(LocalOperationOutcome::new(
            ReturnType::Variable(previous_value),
            RollbackAction::Variable(VariableRollbackAction::Restore { previous }),
        ))
    }

    pub(crate) fn execute_remote_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<(), DatatypeError> {
        let OperationBody::VariableSet(body) = &context.operation().body else {
            return Err(InternalReason::ExecuteOperation(
                "variable cannot execute this remote operation".to_owned(),
            )
            .into_error());
        };

        self.apply_set(&body.value, context.timestamp())?;
        Ok(())
    }

    pub(crate) fn apply_rollback_action(
        &mut self,
        action: VariableRollbackAction,
    ) -> Result<(), DatatypeError> {
        match action {
            VariableRollbackAction::Restore { previous } => {
                self.winning = previous;
                Ok(())
            }
        }
    }

    fn replace(&mut self, value: &[u8], timestamp: &Timestamp) -> VariableSetOutcome {
        let previous = self.winning.replace(VariableState::new(value, timestamp));
        VariableSetOutcome::Applied { previous }
    }
}

impl Debug for VariableCrdt {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VariableCrdt")
            .field("winning", &self.winning)
            .finish()
    }
}

#[cfg(test)]
mod tests_variable_crdt {
    use super::*;
    use crate::{operations::Operation, types::uid::Cuid};

    fn timestamp(lamport: u64, cuid: &str) -> Timestamp {
        Timestamp::new(lamport, &Cuid::try_from(cuid).unwrap())
    }

    fn variable_set_operation(value: &[u8], lamport: u64) -> Operation {
        let mut operation = Operation::new_variable_set(value.into());
        operation.set_lamport(lamport);
        operation
    }

    #[test]
    fn can_start_a_variable_without_a_value() {
        let variable = VariableCrdt::default();

        assert_eq!(variable.value(), None);
        assert_eq!(variable.timestamp(), None);
    }

    #[test]
    fn can_apply_the_first_variable_set() {
        let mut variable = VariableCrdt::default();
        let timestamp = timestamp(1, "0000000000000001");

        let outcome = variable.apply_set(b"first", &timestamp).unwrap();

        assert_eq!(outcome, VariableSetOutcome::Applied { previous: None });
        assert_eq!(variable.value(), Some(b"first".as_slice()));
        assert_eq!(variable.timestamp(), Some(&timestamp));
    }

    #[test]
    fn can_apply_a_newer_variable_set_and_return_the_previous_state() {
        let mut variable = VariableCrdt::default();
        let previous_timestamp = timestamp(1, "0000000000000001");
        let winning_timestamp = timestamp(2, "0000000000000001");
        variable
            .apply_set(b"previous", &previous_timestamp)
            .unwrap();

        let outcome = variable.apply_set(b"winning", &winning_timestamp).unwrap();

        let VariableSetOutcome::Applied {
            previous: Some(previous),
        } = outcome
        else {
            panic!("expected the previous variable state");
        };
        assert_eq!(previous.value(), b"previous");
        assert_eq!(previous.timestamp(), &previous_timestamp);
        assert_eq!(variable.value(), Some(b"winning".as_slice()));
        assert_eq!(variable.timestamp(), Some(&winning_timestamp));
    }

    #[test]
    fn can_ignore_a_stale_variable_set() {
        let mut variable = VariableCrdt::default();
        let winning_timestamp = timestamp(2, "0000000000000001");
        let stale_timestamp = timestamp(1, "0000000000000002");
        variable.apply_set(b"winning", &winning_timestamp).unwrap();

        let outcome = variable.apply_set(b"stale", &stale_timestamp).unwrap();

        assert_eq!(outcome, VariableSetOutcome::Unchanged);
        assert_eq!(variable.value(), Some(b"winning".as_slice()));
        assert_eq!(variable.timestamp(), Some(&winning_timestamp));
    }

    #[test]
    fn can_ignore_a_duplicate_variable_set() {
        let mut variable = VariableCrdt::default();
        let timestamp = timestamp(1, "0000000000000001");
        variable.apply_set(b"same", &timestamp).unwrap();

        let outcome = variable.apply_set(b"same", &timestamp).unwrap();

        assert_eq!(outcome, VariableSetOutcome::Unchanged);
        assert_eq!(variable.value(), Some(b"same".as_slice()));
        assert_eq!(variable.timestamp(), Some(&timestamp));
    }

    #[test]
    fn can_break_variable_set_ties_by_cuid() {
        let mut variable = VariableCrdt::default();
        let lower_timestamp = timestamp(1, "0000000000000001");
        let higher_timestamp = timestamp(1, "0000000000000002");
        variable.apply_set(b"lower", &lower_timestamp).unwrap();

        let outcome = variable.apply_set(b"higher", &higher_timestamp).unwrap();

        assert!(matches!(
            outcome,
            VariableSetOutcome::Applied { previous: Some(_) }
        ));
        assert_eq!(variable.value(), Some(b"higher".as_slice()));
        assert_eq!(variable.timestamp(), Some(&higher_timestamp));
    }

    #[test]
    fn can_reject_different_values_for_the_same_variable_timestamp() {
        let mut variable = VariableCrdt::default();
        let timestamp = timestamp(1, "0000000000000001");
        variable.apply_set(b"winning-secret", &timestamp).unwrap();

        let error = variable
            .apply_set(b"conflicting-secret", &timestamp)
            .unwrap_err();

        assert!(matches!(&error, DatatypeError::Internal(_)));
        assert!(error.to_string().contains("different values for timestamp"));
        assert!(!error.to_string().contains("secret"));
        assert_eq!(variable.value(), Some(b"winning-secret".as_slice()));
        assert_eq!(variable.timestamp(), Some(&timestamp));
    }

    #[test]
    fn can_execute_and_rollback_local_variable_sets_in_reverse_order() {
        let mut variable = VariableCrdt::default();
        let cuid = Cuid::try_from("0000000000000001").unwrap();
        let first_operation = variable_set_operation(b"first", 1);
        let first_context = OperationContext::new(&first_operation, &cuid);

        let first_outcome = variable.execute_local_operation(&first_context).unwrap();
        let (first_return, first_action) = first_outcome.into_parts();
        let RollbackAction::Variable(first_action) = first_action else {
            panic!("expected a variable rollback action");
        };
        assert!(matches!(first_return, ReturnType::Variable(None)));

        let second_operation = variable_set_operation(b"second", 2);
        let second_context = OperationContext::new(&second_operation, &cuid);
        let second_outcome = variable.execute_local_operation(&second_context).unwrap();
        let (second_return, second_action) = second_outcome.into_parts();
        let RollbackAction::Variable(second_action) = second_action else {
            panic!("expected a variable rollback action");
        };
        let ReturnType::Variable(Some(previous_value)) = second_return else {
            panic!("expected the previous variable value");
        };
        let VariableRollbackAction::Restore {
            previous: Some(previous_state),
        } = &second_action
        else {
            panic!("expected the previous variable state");
        };
        assert_eq!(previous_value.as_ref(), b"first");
        assert!(Arc::ptr_eq(&previous_value, &previous_state.value));
        assert_eq!(variable.value(), Some(b"second".as_slice()));
        assert_eq!(variable.timestamp(), Some(second_context.timestamp()));

        variable.apply_rollback_action(second_action).unwrap();
        assert_eq!(variable.value(), Some(b"first".as_slice()));
        assert_eq!(variable.timestamp(), Some(first_context.timestamp()));

        variable.apply_rollback_action(first_action).unwrap();
        assert_eq!(variable.value(), None);
        assert_eq!(variable.timestamp(), None);
    }

    #[test]
    fn can_execute_remote_variable_sets_with_lww_precedence() {
        let mut variable = VariableCrdt::default();
        let cuid = Cuid::try_from("0000000000000001").unwrap();
        let winning_operation = variable_set_operation(b"winning", 2);
        let winning_context = OperationContext::new(&winning_operation, &cuid);
        variable.execute_remote_operation(&winning_context).unwrap();

        let stale_operation = variable_set_operation(b"stale", 1);
        let stale_context = OperationContext::new(&stale_operation, &cuid);
        variable.execute_remote_operation(&stale_context).unwrap();

        assert_eq!(variable.value(), Some(b"winning".as_slice()));
        assert_eq!(variable.timestamp(), Some(winning_context.timestamp()));
    }

    #[test]
    fn can_reject_a_local_variable_set_without_a_newer_timestamp() {
        let mut variable = VariableCrdt::default();
        let cuid = Cuid::try_from("0000000000000001").unwrap();
        let winning_timestamp = Timestamp::new(2, &cuid);
        variable.apply_set(b"winning", &winning_timestamp).unwrap();
        let stale_operation = variable_set_operation(b"winning", 1);
        let stale_context = OperationContext::new(&stale_operation, &cuid);

        let error = variable
            .execute_local_operation(&stale_context)
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("did not advance the winning timestamp")
        );
        assert_eq!(variable.value(), Some(b"winning".as_slice()));
        assert_eq!(variable.timestamp(), Some(&winning_timestamp));
    }

    #[test]
    fn can_reject_non_variable_operations() {
        let mut variable = VariableCrdt::default();
        let operation = Operation::new_counter_increase(1);
        let context = OperationContext::new(&operation, &Cuid::default());

        assert!(variable.execute_local_operation(&context).is_err());
        assert!(variable.execute_remote_operation(&context).is_err());
        assert_eq!(variable.value(), None);
        assert_eq!(variable.timestamp(), None);
    }
}
