use std::{
    fmt::{Debug, Display, Formatter},
    sync::Arc,
};

use crate::{
    DatatypeError,
    datatypes::{
        common::ReturnType,
        crdts::{LocalOperationOutcome, RollbackAction},
    },
    errors::{datatypes::InternalReason, messages::datatype as datatype_msg},
    types::{operation_context::OperationContext, timestamp::Timestamp},
};

pub(super) const INITIAL_VALUE: &[u8] = b"null";

#[derive(PartialEq, Eq)]
pub(crate) struct VariableState {
    value: Arc<[u8]>,
    timestamp: Timestamp,
}

impl VariableState {
    pub(super) fn new(value: &[u8], timestamp: &Timestamp) -> Self {
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
    Applied { previous: VariableState },
    Unchanged,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VariableRollbackAction {
    Restore { previous: VariableState },
}

#[derive(PartialEq, Eq)]
pub struct VariableCrdt {
    pub(super) winning: VariableState,
}

impl Default for VariableCrdt {
    fn default() -> Self {
        Self {
            winning: VariableState::new(INITIAL_VALUE, &Timestamp::initial()),
        }
    }
}

impl VariableCrdt {
    pub(crate) fn fork_for_batch(&self) -> Self {
        Self {
            winning: VariableState {
                value: Arc::clone(&self.winning.value),
                timestamp: self.winning.timestamp.clone(),
            },
        }
    }

    pub(crate) fn value(&self) -> &[u8] {
        self.winning.value()
    }

    /// Returns the winning JSON payload as a shared handle, so a reader can drop the
    /// datatype lock before decoding without copying the payload bytes.
    pub(crate) fn shared_value(&self) -> Arc<[u8]> {
        Arc::clone(&self.winning.value)
    }

    pub(crate) fn timestamp(&self) -> &Timestamp {
        self.winning.timestamp()
    }

    pub(super) fn apply_set(
        &mut self,
        value: &[u8],
        context: &OperationContext<'_>,
    ) -> Result<VariableSetOutcome, DatatypeError> {
        if context.should_apply_lww(
            self.winning.timestamp(),
            Some(self.winning.value()),
            Some(value),
        )? {
            Ok(self.replace(value, context.timestamp()))
        } else {
            Ok(VariableSetOutcome::Unchanged)
        }
    }

    pub(crate) fn execute_local_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<LocalOperationOutcome, DatatypeError> {
        let value = context.variable_input()?;

        let VariableSetOutcome::Applied { previous } = self.apply_set(value, context)? else {
            return Err(InternalReason::ExecuteOperation(
                datatype_msg::LOCAL_TIMESTAMP_NOT_ADVANCED.to_owned(),
            )
            .into_error());
        };
        let previous_value = Arc::clone(&previous.value);

        Ok(LocalOperationOutcome::new(
            ReturnType::Variable(previous_value),
            RollbackAction::Variable(VariableRollbackAction::Restore { previous }),
        ))
    }

    pub(crate) fn execute_remote_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<(), DatatypeError> {
        let value = context.variable_input()?;

        self.apply_set(value, context)?;
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
        let previous = std::mem::replace(&mut self.winning, VariableState::new(value, timestamp));
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

impl Display for VariableCrdt {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "(size:{}, timestamp:{})",
            self.value().len(),
            self.timestamp()
        )
    }
}
