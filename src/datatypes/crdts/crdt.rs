use derive_more::Display;

use super::{
    counter_crdt::CounterCrdt,
    execution::{LocalOperationOutcome, RollbackAction},
    map_crdt::MapCrdt,
    snapshot::SnapshotCodec,
    variable_crdt::VariableCrdt,
};
use crate::{
    DataType, DatatypeError,
    errors::{datatypes::InternalReason, messages::datatype as msg},
    types::operation_context::OperationContext,
};

#[derive(Debug, Display)]
pub enum Crdt {
    Counter(CounterCrdt),
    Variable(VariableCrdt),
    Map(MapCrdt),
}

impl Crdt {
    /// Creates an isolated batch candidate in O(1), sharing immutable payloads.
    /// Never clone CRDT collections here; new datatypes must provide an O(1) fork.
    /// Batch writes must not defer a full collection copy to copy-on-write either.
    pub(crate) fn fork_for_batch(&self) -> Self {
        match self {
            Self::Counter(counter) => Self::Counter(counter.fork_for_batch()),
            Self::Variable(variable) => Self::Variable(variable.fork_for_batch()),
            Self::Map(map) => Self::Map(map.fork_for_batch()),
        }
    }

    pub fn new(r#type: DataType) -> Self {
        match r#type {
            DataType::Counter => Crdt::Counter(CounterCrdt::default()),
            DataType::Variable => Crdt::Variable(VariableCrdt::default()),
            DataType::Map => Crdt::Map(MapCrdt::default()),
        }
    }

    pub fn as_counter(&self) -> Option<&CounterCrdt> {
        match self {
            Self::Counter(counter) => Some(counter),
            _ => None,
        }
    }

    pub fn as_variable(&self) -> Option<&VariableCrdt> {
        match self {
            Self::Variable(variable) => Some(variable),
            _ => None,
        }
    }

    #[cfg(test)]
    pub fn as_map(&self) -> Option<&MapCrdt> {
        match self {
            Self::Map(map) => Some(map),
            _ => None,
        }
    }

    pub(crate) fn execute_local_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<LocalOperationOutcome, DatatypeError> {
        match self {
            Crdt::Counter(c) => c.execute_local_operation(context),
            Crdt::Variable(v) => v.execute_local_operation(context),
            Crdt::Map(m) => m.execute_local_operation(context),
        }
    }

    pub(crate) fn execute_remote_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<(), DatatypeError> {
        match self {
            Crdt::Counter(c) => c.execute_remote_operation(context),
            Crdt::Variable(v) => v.execute_remote_operation(context),
            Crdt::Map(m) => m.execute_remote_operation(context),
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
            (Crdt::Map(m), RollbackAction::Map(action)) => m.apply_rollback_action(action),
            _ => Err(
                InternalReason::ExecuteOperation(msg::ROLLBACK_ACTION_MISMATCH.to_owned())
                    .into_error(),
            ),
        }
    }

    pub fn serialize(&self) -> Box<[u8]> {
        match self {
            Self::Counter(c) => c.encode_snapshot(),
            Self::Variable(v) => v.encode_snapshot(),
            Self::Map(m) => m.encode_snapshot(),
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
            Self::Map(m) => {
                *m = MapCrdt::decode_snapshot(serialized)?;
                Ok(())
            }
        }
    }
}
