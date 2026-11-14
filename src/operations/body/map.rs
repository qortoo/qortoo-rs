use std::fmt::{Debug, Formatter};

use derive_more::Display;

use super::OperationBody;
use crate::operations::{MemoryMeasurable, Operation};

#[derive(Clone, Display, PartialEq, Eq)]
#[display("(key_size:{}, value_size:{})", key.len(), value.len())]
pub struct MapPutBody {
    pub key: String,
    pub value: Box<[u8]>,
}

#[derive(Clone, Display, PartialEq, Eq)]
#[display("(key_size:{})", key.len())]
pub struct MapRemoveBody {
    pub key: String,
}

impl Debug for MapPutBody {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapPutBody")
            .field("key_size", &self.key.len())
            .field("value_size", &self.value.len())
            .finish()
    }
}

impl Debug for MapRemoveBody {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MapRemoveBody")
            .field("key_size", &self.key.len())
            .finish()
    }
}

impl MemoryMeasurable for MapPutBody {
    fn size(&self) -> u64 {
        (self.key.len() + self.value.len()) as u64
    }
}

impl MemoryMeasurable for MapRemoveBody {
    fn size(&self) -> u64 {
        self.key.len() as u64
    }
}

impl Operation {
    pub fn new_map_put(key: String, value: Box<[u8]>) -> Self {
        Self::new(OperationBody::MapPut(MapPutBody { key, value }))
    }

    pub fn new_map_remove(key: String) -> Self {
        Self::new(OperationBody::MapRemove(MapRemoveBody { key }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn can_measure_map_operations_without_logging_keys_or_values() {
        let key = "한글-secret-key";
        let value = br#""secret-value""#;
        let operations = [
            Operation::new_map_put(key.into(), value.as_slice().into()),
            Operation::new_map_remove(key.into()),
        ];
        assert_eq!(operations[0].body.size(), (key.len() + value.len()) as u64);
        assert_eq!(operations[1].body.size(), key.len() as u64);
        for operation in operations {
            for text in [
                format!("{operation}"),
                format!("{operation:?}"),
                format!("{:?}", operation.body),
            ] {
                assert!(!text.contains("secret"));
            }
        }
    }
}
