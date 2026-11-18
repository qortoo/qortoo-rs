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
    utils::json::validate_json_str,
};

pub(super) const INITIAL_VALUE: &[u8] = b"null";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VariableSetOutcome {
    Applied { previous: VariableCrdt },
    Unchanged,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum VariableRollbackAction {
    Restore { previous: VariableCrdt },
}

#[derive(PartialEq, Eq)]
pub struct VariableCrdt {
    value: Arc<[u8]>,
    timestamp: Timestamp,
}

impl Default for VariableCrdt {
    fn default() -> Self {
        Self::new(INITIAL_VALUE, &Timestamp::initial())
    }
}

impl VariableCrdt {
    pub(super) fn new(value: &[u8], timestamp: &Timestamp) -> Self {
        Self {
            value: value.into(),
            timestamp: timestamp.clone(),
        }
    }

    pub(super) fn validate_state(&self) -> Option<&str> {
        let value = validate_json_str(self.value()).ok()?;
        if self.timestamp.lamport() > 0
            || (self.timestamp.is_initial() && self.value() == INITIAL_VALUE)
        {
            return Some(value);
        }
        None
    }

    pub(crate) fn fork_for_batch(&self) -> Self {
        Self {
            value: Arc::clone(&self.value),
            timestamp: self.timestamp.clone(),
        }
    }

    pub(crate) fn value(&self) -> &[u8] {
        &self.value
    }

    /// Returns the winning JSON payload as a shared handle, so a reader can drop the
    /// datatype lock before decoding without copying the payload bytes.
    pub(crate) fn shared_value(&self) -> Arc<[u8]> {
        Arc::clone(&self.value)
    }

    pub(crate) fn timestamp(&self) -> &Timestamp {
        &self.timestamp
    }

    pub(super) fn apply_set(
        &mut self,
        value: &[u8],
        context: &OperationContext<'_>,
    ) -> Result<VariableSetOutcome, DatatypeError> {
        if context.should_apply_lww(self.timestamp(), Some(self.value()), Some(value))? {
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
                *self = previous;
                Ok(())
            }
        }
    }

    fn replace(&mut self, value: &[u8], timestamp: &Timestamp) -> VariableSetOutcome {
        let previous = std::mem::replace(self, Self::new(value, timestamp));
        VariableSetOutcome::Applied { previous }
    }
}

impl Debug for VariableCrdt {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VariableCrdt")
            .field("value_size", &self.value.len())
            .field("timestamp", &self.timestamp)
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
