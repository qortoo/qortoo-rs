use derive_more::Display;

#[cfg(test)]
use crate::operations::body::OperationBody;
use crate::{
    DataType, DatatypeError,
    datatypes::{common::ReturnType, crdts::counter_crdt::CounterCrdt},
    errors::datatypes::InternalReason,
    operations::Operation,
};

pub mod counter_crdt;

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

    pub fn execute_local_operation(&mut self, op: &Operation) -> Result<ReturnType, DatatypeError> {
        #[cfg(test)]
        {
            if let OperationBody::Delay4Test(body) = &op.body {
                return match body.run() {
                    Ok(_) => Ok(ReturnType::None),
                    Err(_) => Err(InternalReason::ExecuteOperation(format!("{body}")).into_error()),
                };
            }
        }
        match self {
            Crdt::Counter(c) => c.execute_common_operation(op),
        }
    }

    pub fn execute_remote_operation(
        &mut self,
        op: &Operation,
    ) -> Result<ReturnType, DatatypeError> {
        #[cfg(test)]
        {
            if let OperationBody::Delay4Test(body) = &op.body {
                return match body.run() {
                    Ok(_) => Ok(ReturnType::None),
                    Err(_) => Err(InternalReason::ExecuteOperation(format!("{body}")).into_error()),
                };
            }
        }
        match self {
            Crdt::Counter(c) => c.execute_common_operation(op),
        }
    }

    pub fn execute_inverse_operation(
        &mut self,
        op: &Operation,
    ) -> Result<ReturnType, DatatypeError> {
        match self {
            Crdt::Counter(c) => c.execute_inverse_operation(op),
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
mod tests_crdts {
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
