use std::fmt::Debug;

use crate::{
    DatatypeState,
    operations::{Operation, transaction::Transaction},
    types::operation_id::OperationId,
};

pub struct TxRecord {
    pub pending: Option<Transaction>,
    pub rollback_op_id: OperationId,
    pub rollback_state: DatatypeState,
}

impl TxRecord {
    pub fn new(state: DatatypeState, op_id: OperationId) -> Self {
        Self {
            pending: None,
            rollback_op_id: op_id,
            rollback_state: state,
        }
    }

    /// Appends a successfully executed operation to the active transaction.
    /// If no transaction is active, a new one is started and the given `op_id` and `state`
    /// are saved as the rollback point.
    /// Returns `true` if a new transaction was started.
    pub fn record_operation(
        &mut self,
        op_id: &OperationId,
        state: DatatypeState,
        op: Operation,
    ) -> bool {
        let is_new = self.pending.is_none();
        if is_new {
            self.rollback_op_id = op_id.clone();
            self.rollback_state = state;
            self.pending = Some(Transaction::new(&op_id.cuid, op_id.cseq + 1));
        }
        self.pending.as_mut().unwrap().push_operation(op);
        is_new
    }
}

impl Debug for TxRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entry(&"rollback_state", &self.rollback_state)
            .finish()
    }
}

#[cfg(test)]
mod tests_tx_record {
    use tracing::info;

    use crate::datatypes::tx_record::TxRecord;

    #[test]
    fn can_debug_tx_record() {
        let tx_record = TxRecord::new(Default::default(), Default::default());
        info!("{:?}", tx_record);
    }
}
