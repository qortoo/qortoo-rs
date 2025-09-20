use std::fmt::Debug;

use crate::{
    DatatypeState,
    datatypes::{crdts::Crdt, mutable::OperationalDatatype},
    types::operation_id::OperationId,
};

pub struct Rollback {
    pub shadow_crdt: Crdt,
    pub op_id: OperationId,
    pub state: DatatypeState,
}

impl Rollback {
    pub fn new(crdt: Crdt, state: DatatypeState, op_id: OperationId) -> Self {
        Self {
            shadow_crdt: crdt,
            op_id,
            state,
        }
    }

    pub fn get_operational_datatype(&mut self) -> OperationalDatatype {
        OperationalDatatype {
            crdt: &mut self.shadow_crdt,
            op_id: &mut self.op_id,
        }
    }
}

impl Debug for Rollback {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entry(&"shadow", &self.shadow_crdt)
            .entry(&"state", &self.state)
            .finish()
    }
}

#[cfg(test)]
mod tests_rollback {
    use crate::{
        DataType,
        datatypes::{crdts::Crdt, rollback::Rollback},
    };

    #[test]
    fn can_debug_rollback() {
        let rollback = Rollback::new(
            Crdt::new(DataType::Counter),
            Default::default(),
            Default::default(),
        );
        println!("{:?}", rollback);
    }
}
