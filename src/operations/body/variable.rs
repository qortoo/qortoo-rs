use std::fmt::{Debug, Formatter};

use derive_more::Display;

use super::OperationBody;
use crate::operations::{MemoryMeasurable, Operation};

#[derive(Clone, Display, PartialEq, Eq)]
#[display("(size:{})", value.len())]
pub struct VariableSetBody {
    pub value: Box<[u8]>,
}

impl VariableSetBody {
    pub fn new(value: Box<[u8]>) -> Self {
        Self { value }
    }
}

impl Debug for VariableSetBody {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VariableSetBody")
            .field("size", &self.value.len())
            .finish()
    }
}

impl MemoryMeasurable for VariableSetBody {
    fn size(&self) -> u64 {
        self.value.len() as u64
    }
}

impl Operation {
    pub fn new_variable_set(value: Box<[u8]>) -> Self {
        Self::new(OperationBody::VariableSet(VariableSetBody::new(value)))
    }
}

#[cfg(test)]
mod tests_variable_set_body {
    use super::*;

    #[test]
    fn can_create_a_variable_set_operation() {
        let payload = br#"{"name":"qortoo"}"#.to_vec().into_boxed_slice();
        let payload_size = payload.len() as u64;
        let op = Operation::new_variable_set(payload.clone());
        assert!(!format!("{op:?}").contains("qortoo"));
        assert_eq!(op.body.size(), payload_size);

        let OperationBody::VariableSet(body) = &op.body else {
            panic!("expected a variable set operation");
        };
        assert_eq!(body.value, payload);
    }

    #[test]
    fn can_hide_variable_payload_from_display_and_debug() {
        let payload = br#"{"secret":"do-not-log"}"#.to_vec().into_boxed_slice();
        let payload_size = payload.len();
        let body = VariableSetBody::new(payload);

        assert_eq!(format!("{body}"), format!("(size:{payload_size})"));
        assert_eq!(
            format!("{body:?}"),
            format!("VariableSetBody {{ size: {payload_size} }}")
        );

        let operation_body = OperationBody::VariableSet(body);
        let display = format!("VariableSet(size:{payload_size})");
        assert_eq!(format!("{operation_body}"), display);

        let debug = format!("{operation_body:?}");
        assert_eq!(debug, display);
        assert!(!debug.contains("do-not-log"));
    }
}
