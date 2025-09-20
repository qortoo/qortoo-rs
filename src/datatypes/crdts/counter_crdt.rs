use crate::{
    DatatypeError,
    datatypes::common::ReturnType,
    operations::{Operation, body::OperationBody},
};

#[derive(Debug, Default, Clone)]
pub struct CounterCrdt {
    value: i64,
}

impl CounterCrdt {
    pub fn increase_by(&mut self, value: i64) -> i64 {
        self.value += value;
        self.value
    }

    pub fn value(&self) -> i64 {
        self.value
    }

    pub fn execute_local_operation(&mut self, op: &Operation) -> Result<ReturnType, DatatypeError> {
        match op.body {
            OperationBody::CounterIncrease(ref body) => {
                let ret = self.increase_by(body.delta);
                Ok(ReturnType::Counter(ret))
            }
            #[allow(unreachable_patterns)]
            _ => unimplemented!(),
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
    use tracing::info;

    use crate::datatypes::crdts::counter_crdt::CounterCrdt;

    #[test]
    fn can_new_and_increase_counter() {
        let mut counter = CounterCrdt::default();
        counter.increase_by(1);
        counter.increase_by(-2);
        assert_eq!(counter.value(), -1);
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
}
