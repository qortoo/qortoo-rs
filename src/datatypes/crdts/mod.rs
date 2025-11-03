#[cfg(test)]
use crate::operations::body::OperationBody;
use crate::{
    DataType, DatatypeError,
    datatypes::{common::ReturnType, crdts::counter_crdt::CounterCrdt},
    errors::with_err_out,
    operations::Operation,
};

pub mod counter_crdt;

#[derive(Debug, Clone)]
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
                    Err(_) => Err(DatatypeError::FailedToExecuteOperation(format!("{body}"))),
                };
            }
        }
        match self {
            Crdt::Counter(c) => c.execute_local_operation(op),
        }
    }

    pub fn serialize(&self) -> Box<[u8]> {
        match self {
            Self::Counter(c) => Box::new(c.to_bytes()),
        }
    }

    pub fn deserialize(&mut self, serialized: &[u8]) {
        match self {
            Self::Counter(c) => {
                if serialized.len() != 8 {
                    with_err_out!(DatatypeError::FailedToDeserialize(format!(
                        "counter crdt: {serialized:?}, and will recover to counter value 0"
                    )));
                    *c = CounterCrdt::default();
                    return;
                }
                let mut array = [0u8; 8];
                array.copy_from_slice(serialized);
                *c = CounterCrdt::from_bytes(&array)
            }
        }
    }
}

#[cfg(test)]
mod tests_crdts {
    use crate::{
        DataType,
        datatypes::crdts::{Crdt, counter_crdt::CounterCrdt},
    };

    #[test]
    fn can_serialize_and_deserialize() {
        let mut counter = CounterCrdt::default();
        counter.increase_by(100);
        let crdt1 = Crdt::Counter(counter);

        let mut crdt2 = Crdt::new(DataType::Counter);
        let serialized = crdt1.serialize();
        crdt2.deserialize(&serialized);

        let Crdt::Counter(c) = &crdt2;
        assert_eq!(c.value(), 100);
        crdt2.deserialize("{}".as_bytes());

        let Crdt::Counter(c) = &crdt2;
        assert_eq!(c.value(), 0);
    }
}
