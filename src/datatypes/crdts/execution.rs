use super::{
    counter_crdt::CounterRollbackAction, map_crdt::MapRollbackAction,
    variable_crdt::VariableRollbackAction,
};
use crate::datatypes::common::ReturnType;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RollbackAction {
    Counter(CounterRollbackAction),
    Variable(VariableRollbackAction),
    Map(MapRollbackAction),
}

#[derive(Debug)]
pub(crate) enum LocalOperationOutcome {
    Applied {
        return_value: ReturnType,
        rollback_action: RollbackAction,
    },
    NoOp {
        return_value: ReturnType,
    },
}

impl LocalOperationOutcome {
    pub(crate) fn new(return_value: ReturnType, rollback_action: RollbackAction) -> Self {
        Self::Applied {
            return_value,
            rollback_action,
        }
    }

    #[cfg(test)]
    pub(crate) fn into_parts(self) -> (ReturnType, RollbackAction) {
        let Self::Applied {
            return_value,
            rollback_action,
        } = self
        else {
            panic!("expected an applied operation");
        };
        (return_value, rollback_action)
    }
}
