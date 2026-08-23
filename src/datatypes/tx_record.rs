use std::fmt::Debug;

use crate::{
    DatatypeState,
    datatypes::crdts::RollbackAction,
    operations::{Operation, transaction::Transaction},
    types::operation_id::OperationId,
};

pub struct TxRecord {
    pub pending: Option<Transaction>,
    pub rollback_op_id: OperationId,
    pub rollback_state: DatatypeState,
    rollback_actions: Vec<RollbackAction>,
}

impl TxRecord {
    pub fn new(state: DatatypeState, op_id: OperationId) -> Self {
        Self {
            pending: None,
            rollback_op_id: op_id,
            rollback_state: state,
            rollback_actions: Vec::new(),
        }
    }

    /// Appends a successfully executed operation and its local rollback action to the active
    /// transaction.
    /// If no transaction is active, a new one is started and the given `op_id` and `state`
    /// are saved as the rollback point.
    /// Returns `true` if a new transaction was started.
    pub fn record_operation(
        &mut self,
        op_id: &OperationId,
        state: DatatypeState,
        op: Operation,
        rollback_action: RollbackAction,
    ) -> bool {
        let is_new = self.pending.is_none();
        if is_new {
            debug_assert!(self.rollback_actions.is_empty());
            self.rollback_op_id = op_id.clone();
            self.rollback_state = state;
            self.pending = Some(Transaction::new(&op_id.cuid, op_id.cseq + 1));
        }
        self.pending.as_mut().unwrap().push_operation(op);
        self.rollback_actions.push(rollback_action);
        is_new
    }

    pub fn take_rollback_actions(&mut self) -> Vec<RollbackAction> {
        std::mem::take(&mut self.rollback_actions)
    }

    pub fn discard_rollback_actions(&mut self) {
        self.rollback_actions.clear();
    }

    #[cfg(test)]
    pub fn rollback_action_count(&self) -> usize {
        self.rollback_actions.len()
    }
}

impl Debug for TxRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entry(&"rollback_state", &self.rollback_state)
            .entry(&"rollback_action_count", &self.rollback_actions.len())
            .finish()
    }
}

#[cfg(test)]
mod tests_tx_record {
    use tracing::info;

    use crate::{
        datatypes::{
            crdts::{RollbackAction, counter_crdt::CounterRollbackAction},
            tx_record::TxRecord,
        },
        operations::Operation,
        types::operation_id::OperationId,
    };

    #[test]
    fn can_debug_tx_record() {
        let tx_record = TxRecord::new(Default::default(), Default::default());
        info!("{:?}", tx_record);
    }

    #[test]
    fn can_record_and_take_rollback_actions() {
        let op_id = OperationId::default();
        let mut tx_record = TxRecord::new(Default::default(), op_id.clone());

        assert!(tx_record.record_operation(
            &op_id,
            Default::default(),
            Operation::new_counter_increase(3),
            RollbackAction::Counter(CounterRollbackAction::Increase { delta: -3 }),
        ));
        assert!(!tx_record.record_operation(
            &op_id,
            Default::default(),
            Operation::new_counter_increase(5),
            RollbackAction::Counter(CounterRollbackAction::Increase { delta: -5 }),
        ));

        assert_eq!(tx_record.pending.as_ref().unwrap().operations.len(), 2);
        assert_eq!(
            tx_record.take_rollback_actions(),
            vec![
                RollbackAction::Counter(CounterRollbackAction::Increase { delta: -3 }),
                RollbackAction::Counter(CounterRollbackAction::Increase { delta: -5 }),
            ]
        );
        assert!(tx_record.take_rollback_actions().is_empty());
    }
}
