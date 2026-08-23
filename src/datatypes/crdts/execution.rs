use super::counter_crdt::CounterRollbackAction;
use crate::datatypes::common::ReturnType;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RollbackAction {
    Counter(CounterRollbackAction),
}

#[derive(Debug)]
pub(crate) struct LocalOperationOutcome {
    return_value: ReturnType,
    rollback_action: RollbackAction,
}

impl LocalOperationOutcome {
    pub(crate) fn new(return_value: ReturnType, rollback_action: RollbackAction) -> Self {
        Self {
            return_value,
            rollback_action,
        }
    }

    pub(crate) fn into_parts(self) -> (ReturnType, RollbackAction) {
        (self.return_value, self.rollback_action)
    }
}
