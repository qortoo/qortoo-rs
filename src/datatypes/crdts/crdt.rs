use derive_more::Display;

use super::{
    counter_crdt::CounterCrdt,
    execution::{LocalOperationOutcome, RollbackAction},
    map_crdt::MapCrdt,
    snapshot::SnapshotCodec,
    variable_crdt::VariableCrdt,
};
use crate::{
    DataType, DatatypeError, SnapshotDecodeLimits, SnapshotWriteOptions,
    errors::{datatypes::InternalReason, messages::datatype as msg},
    operations::transaction::Transaction,
    types::operation_context::OperationContext,
};

#[derive(Debug, Display)]
pub enum Crdt {
    Counter(CounterCrdt),
    Variable(VariableCrdt),
    Map(MapCrdt),
}

impl Crdt {
    pub(crate) fn logical_state_bytes(&self) -> u64 {
        match self {
            Self::Counter(_) => size_of::<i64>() as u64,
            Self::Variable(variable) => variable.value().len() as u64,
            Self::Map(map) => map.logical_state_bytes(),
        }
    }

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

    /// Applies operations in order and advances the supplied clock after each success.
    /// On error, earlier operations and their clock updates remain applied.
    pub(crate) fn execute_remote_transaction(
        &mut self,
        tx: &Transaction,
        lamport: &mut u64,
    ) -> Result<(), DatatypeError> {
        for op in tx.iter() {
            let context = OperationContext::try_new(op, &tx.cuid)?;
            self.execute_remote_operation(&context)?;
            *lamport = (*lamport).max(op.lamport);
        }
        Ok(())
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

    pub fn serialize(&self, options: SnapshotWriteOptions) -> Result<Box<[u8]>, DatatypeError> {
        match self {
            Self::Counter(c) => c.encode_snapshot(options),
            Self::Variable(v) => v.encode_snapshot(options),
            Self::Map(m) => m.encode_snapshot(options),
        }
        .map_err(DatatypeError::SnapshotEncode)
    }

    pub fn deserialize(
        &mut self,
        serialized: &[u8],
        limits: SnapshotDecodeLimits,
    ) -> Result<(), DatatypeError> {
        let candidate = match self {
            Self::Counter(_) => CounterCrdt::decode_snapshot(serialized, limits).map(Self::Counter),
            Self::Variable(_) => {
                VariableCrdt::decode_snapshot(serialized, limits).map(Self::Variable)
            }
            Self::Map(_) => MapCrdt::decode_snapshot(serialized, limits).map(Self::Map),
        }
        .map_err(DatatypeError::SnapshotDecode)?;
        *self = candidate;
        Ok(())
    }
}
