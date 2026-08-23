use derive_more::Display;

use super::{
    counter_crdt::CounterCrdt,
    execution::{LocalOperationOutcome, RollbackAction},
};
use crate::{
    DataType, DatatypeError, errors::datatypes::InternalReason,
    types::operation_context::OperationContext,
};

#[derive(Debug, Clone, Display)]
pub enum Crdt {
    Counter(CounterCrdt),
}

impl Crdt {
    pub fn new(r#type: DataType) -> Self {
        match r#type {
            DataType::Counter => Crdt::Counter(CounterCrdt::default()),
            _ => unreachable!("invalid type"),
        }
    }

    pub(crate) fn execute_local_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<LocalOperationOutcome, DatatypeError> {
        match self {
            Crdt::Counter(c) => c.execute_local_operation(context),
        }
    }

    pub(crate) fn execute_remote_operation(
        &mut self,
        context: &OperationContext<'_>,
    ) -> Result<(), DatatypeError> {
        match self {
            Crdt::Counter(c) => c.execute_remote_operation(context),
        }
    }

    pub(crate) fn apply_rollback_action(
        &mut self,
        action: RollbackAction,
    ) -> Result<(), DatatypeError> {
        match (self, action) {
            (Crdt::Counter(c), RollbackAction::Counter(action)) => c.apply_rollback_action(action),
            #[allow(unreachable_patterns)]
            _ => Err(InternalReason::ExecuteOperation(
                "rollback action does not match crdt".to_owned(),
            )
            .into_error()),
        }
    }

    pub fn serialize(&self) -> Box<[u8]> {
        match self {
            Self::Counter(c) => Box::new(c.to_bytes()),
        }
    }

    pub fn deserialize(&mut self, serialized: &[u8]) -> Result<(), DatatypeError> {
        match self {
            Self::Counter(c) => {
                if serialized.len() != 8 {
                    return Err(InternalReason::Deserialize(format!(
                        "counter crdt: expected 8 bytes, got {}",
                        serialized.len()
                    ))
                    .into_error());
                }
                let mut array = [0u8; 8];
                array.copy_from_slice(serialized);
                *c = CounterCrdt::from_bytes(&array);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests_crdt {
    use crate::{
        DataType, DatatypeError,
        datatypes::crdts::{Crdt, counter_crdt::CounterCrdt},
    };

    #[test]
    fn can_serialize_and_deserialize() {
        let mut counter = CounterCrdt::default();
        counter.increase_by(100);
        let crdt1 = Crdt::Counter(counter);

        let mut crdt2 = Crdt::new(DataType::Counter);
        let serialized = crdt1.serialize();
        crdt2.deserialize(&serialized).unwrap();

        let Crdt::Counter(c) = &crdt2;
        assert_eq!(c.value(), 100);

        // Invalid input returns Err; counter value must not change.
        assert!(matches!(
            crdt2.deserialize("{}".as_bytes()),
            Err(DatatypeError::Internal(_))
        ));
        let Crdt::Counter(c) = &crdt2;
        assert_eq!(c.value(), 100);
    }
}
