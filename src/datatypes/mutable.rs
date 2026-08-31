use std::{collections::BTreeMap, sync::Arc};

use tracing::instrument;

use crate::{
    DatatypeError, DatatypeHandler, DatatypeState, ServerRejectReason,
    datatypes::{
        common::{Attribute, ReturnType},
        crdts::Crdt,
        handler::HandlersManager,
        push_buffer::{MemoryPushBuffer, PushBuffer},
        tx_record::TxRecord,
    },
    errors::{
        datatypes::{DatatypeErrorWithAction, RecoveryAction},
        with_err_out,
    },
    operations::{Operation, body::OperationBody, transaction::Transaction},
    types::{
        checkpoint::CheckPoint, operation_context::OperationContext, operation_id::OperationId,
    },
};

pub(crate) const DATATYPE_ERR_MSG_NO_SNAPSHOT: &str = "no snapshot operation";

#[derive(Debug)]
pub struct MutableDatatype {
    pub attr: Arc<Attribute>,
    pub crdt: Crdt,
    pub op_id: OperationId,
    pub push_buffer: MemoryPushBuffer,
    pub checkpoint: CheckPoint,
    state: DatatypeState,
    tx_record: TxRecord,
    handlers_manager: HandlersManager,
}

impl MutableDatatype {
    pub fn new(
        attr: Arc<Attribute>,
        state: DatatypeState,
        handlers: BTreeMap<usize, DatatypeHandler>,
    ) -> Self {
        let crdt = Crdt::new(attr.r#type);
        let op_id = OperationId::new_with_cuid(&attr.client_common.cuid);
        Self {
            push_buffer: MemoryPushBuffer::new(attr.option.clone()),
            tx_record: TxRecord::new(state, op_id.clone()),
            checkpoint: CheckPoint::default(),
            handlers_manager: HandlersManager::new(attr.clone(), handlers),
            attr,
            crdt,
            state,
            op_id,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.push_buffer = MemoryPushBuffer::new(self.attr.option.clone());
        self.tx_record = TxRecord::new(self.state, self.op_id.clone());
    }

    pub fn disable(&mut self) {
        self.reset();
        self.set_state(DatatypeState::Disabled);
    }

    pub fn apply_snapshot_transaction(
        &mut self,
        tx: Arc<Transaction>,
    ) -> Result<(), DatatypeError> {
        if tx.operations.is_empty() {
            return Err(DatatypeError::ServerRejected(
                ServerRejectReason::ProtocolViolation(DATATYPE_ERR_MSG_NO_SNAPSHOT.to_owned()),
            ));
        }
        let snap_op = &tx.operations[0];
        if let OperationBody::Snapshot(body) = &snap_op.body {
            self.crdt.deserialize(&body.data)?;
            self.op_id.cseq = 0;
            self.op_id.lamport = snap_op.lamport;
            self.reset();
        } else {
            return Err(DatatypeError::ServerRejected(
                ServerRejectReason::ProtocolViolation(DATATYPE_ERR_MSG_NO_SNAPSHOT.to_owned()),
            ));
        }
        Ok(())
    }

    #[instrument(skip_all)]
    pub fn do_rollback(&mut self) {
        if let Some(tx) = self.tx_record.pending.take() {
            let rollback_actions = self.tx_record.take_rollback_actions();
            debug_assert_eq!(tx.operations.len(), rollback_actions.len());
            for action in rollback_actions.into_iter().rev() {
                if let Err(e) = self.crdt.apply_rollback_action(action) {
                    with_err_out!(e);
                }
            }
            self.op_id = self.tx_record.rollback_op_id.clone();
            self.set_state(self.tx_record.rollback_state);
        }
    }

    /// Ends the in-progress transaction.
    ///
    /// Returns `Ok(true)` when a committed transaction was enqueued into the push buffer,
    /// `Ok(false)` when the transaction was rolled back (`committed == false`).
    /// On an enqueue failure, `pending` is restored so that the routed
    /// `RecoveryAction::RollbackTransaction` can undo the transaction, and the error is returned.
    pub fn end_transaction(
        &mut self,
        tag: Option<String>,
        committed: bool,
    ) -> Result<bool, DatatypeErrorWithAction> {
        if !committed {
            self.do_rollback();
            return Ok(false);
        }

        if let Some(mut tx) = self.tx_record.pending.take() {
            tx.set_tag(tag);
            let tx = Arc::new(tx);
            if tx.cuid == self.op_id.cuid {
                if let Err(err) = self.push_buffer.enqueue(tx.clone()) {
                    // The clone passed to enqueue is dropped on failure, so this Arc is unique
                    // again. Restore the wire transaction while retaining its rollback actions
                    // so RecoveryAction::RollbackTransaction can undo the local changes.
                    self.tx_record.pending = Arc::try_unwrap(tx).ok();
                    return Err(err);
                }
            }
            self.tx_record.discard_rollback_actions();
        }
        Ok(true)
    }

    /// Applies the datatype-lifecycle side effect of a routed error.
    ///
    /// Single dispatch point for `RecoveryAction`, shared by the event-loop path
    /// (`WiredDatatype::handle_error`) and the transaction-commit path
    /// (`TransactionalDatatype::end_transaction`).
    pub fn apply_action(&mut self, recovery: RecoveryAction) {
        match recovery {
            RecoveryAction::NotifyOnly | RecoveryAction::RetryWithBackOff => {}
            RecoveryAction::RollbackTransaction => self.do_rollback(),
            RecoveryAction::Resubscribe | RecoveryAction::ResubscribeWithBackOff => {
                self.reset();
                self.set_state(DatatypeState::SubscribingOrCreating);
            }
            RecoveryAction::Disable => self.disable(),
        }
    }

    pub fn execute_remote_transaction(
        &mut self,
        tx: Arc<Transaction>,
    ) -> Result<(), DatatypeError> {
        for op in tx.iter() {
            let context = OperationContext::try_new(op, &tx.cuid)?;
            self.op_id.lamport = self.op_id.lamport.max(op.lamport);
            self.crdt.execute_remote_operation(&context)?;
        }
        Ok(())
    }

    #[instrument(skip_all)]
    pub fn execute_local_operation(
        &mut self,
        mut op: Operation,
    ) -> Result<ReturnType, DatatypeError> {
        op.set_lamport(self.op_id.lamport + 1);
        let outcome = {
            let context = OperationContext::try_new(&op, &self.op_id.cuid)?;
            self.crdt.execute_local_operation(&context)?
        };
        let (return_value, rollback_action) = outcome.into_parts();
        let is_new_tx =
            self.tx_record
                .record_operation(&self.op_id, self.state, op, rollback_action);
        self.op_id.next(is_new_tx);
        Ok(return_value)
    }

    pub fn new_snapshot_operation(&self) -> Operation {
        let data = self.crdt.serialize();
        let mut snap_op = Operation::new_snapshot(data);
        snap_op.lamport = self.op_id.lamport;
        snap_op
    }

    pub fn set_handler(&mut self, priority: usize, handler: DatatypeHandler) {
        self.handlers_manager.set_handler(priority, handler);
    }

    pub fn unset_handler(&mut self, priority: usize) -> Option<DatatypeHandler> {
        self.handlers_manager.unset_handler(priority)
    }

    pub fn get_state(&self) -> DatatypeState {
        self.state
    }

    pub fn set_state(&mut self, new_state: DatatypeState) {
        let old_state = self.state;
        if old_state != new_state {
            self.state = new_state;
            self.handlers_manager
                .notify_state_change(old_state, new_state);
            if new_state == DatatypeState::Disabled {
                self.attr.detach_datatype_if_same_instance();
            }
        }
    }

    pub fn call_error_handler(&self, err: DatatypeError) {
        self.handlers_manager.notify_error(err)
    }
}

#[cfg(test)]
mod tests_mutable_datatype {
    use tracing::instrument;

    use crate::{
        DataType,
        datatypes::{common::new_attribute, transactional::TransactionalDatatype},
        operations::{Operation, transaction::Transaction},
        types::uid::Cuid,
    };

    #[test]
    #[instrument]
    fn can_preserve_transaction_state_when_operation_execution_fails() {
        let attr = new_attribute!(DataType::Counter);
        let tx_dt = TransactionalDatatype::new_arc(attr, Default::default(), Default::default());
        {
            let mutable = tx_dt.mutable.write();
            assert_eq!(0, mutable.op_id.cseq);
            assert!(mutable.tx_record.pending.is_none());
            assert_eq!(mutable.tx_record.rollback_action_count(), 0);
            assert_eq!(mutable.op_id, mutable.tx_record.rollback_op_id);
        }

        let op1 = Operation::new_counter_increase(1);
        let result1 = tx_dt.execute_local_operation_as_tx(Default::default(), op1);
        assert!(result1.is_ok());
        {
            let mutable = tx_dt.mutable.write();
            let counter = mutable.crdt.as_counter().expect("expected a counter crdt");
            assert_eq!(counter.value(), 1);
            assert_eq!(1, mutable.op_id.cseq);
            assert!(mutable.tx_record.pending.is_none());
            assert_eq!(mutable.tx_record.rollback_action_count(), 0);
            assert_eq!(
                mutable.op_id.cseq,
                mutable.tx_record.rollback_op_id.cseq + 1
            );
        }

        let op2 = Operation::new_snapshot(Vec::new().into_boxed_slice());
        let result2 = tx_dt.execute_local_operation_as_tx(Default::default(), op2);
        assert!(result2.is_err());
        {
            let mutable = tx_dt.mutable.write();
            let counter = mutable.crdt.as_counter().expect("expected a counter crdt");
            assert_eq!(counter.value(), 1);
            assert_eq!(1, mutable.op_id.cseq);
            assert!(mutable.tx_record.pending.is_none());
            assert_eq!(mutable.tx_record.rollback_action_count(), 0);
        }
    }

    #[test]
    #[instrument]
    fn can_manage_rollback_action_lifetimes() {
        let attr = new_attribute!(DataType::Counter);
        let mut mutable = super::MutableDatatype::new(attr, Default::default(), Default::default());

        mutable
            .execute_local_operation(Operation::new_counter_increase(3))
            .unwrap();
        assert_eq!(mutable.tx_record.rollback_action_count(), 1);
        assert!(mutable.end_transaction(None, true).unwrap());
        assert_eq!(mutable.tx_record.rollback_action_count(), 0);

        mutable
            .execute_local_operation(Operation::new_counter_increase(5))
            .unwrap();
        assert_eq!(mutable.tx_record.rollback_action_count(), 1);
        assert!(!mutable.end_transaction(None, false).unwrap());
        assert_eq!(mutable.tx_record.rollback_action_count(), 0);
        let counter = mutable.crdt.as_counter().expect("expected a counter crdt");
        assert_eq!(counter.value(), 3);

        mutable
            .execute_local_operation(Operation::new_counter_increase(7))
            .unwrap();
        assert_eq!(mutable.tx_record.rollback_action_count(), 1);
        mutable.reset();
        assert!(mutable.tx_record.pending.is_none());
        assert_eq!(mutable.tx_record.rollback_action_count(), 0);
    }

    #[test]
    #[instrument]
    fn can_reject_a_remote_counter_operation_with_zero_lamport() {
        let attr = new_attribute!(DataType::Counter);
        let mut mutable = super::MutableDatatype::new(attr, Default::default(), Default::default());
        let cuid = Cuid::try_from("0000000000000001").unwrap();
        let mut transaction = Transaction::new(&cuid, 1);
        transaction.push_operation(Operation::new_counter_increase(1));

        let error = mutable
            .execute_remote_transaction(std::sync::Arc::new(transaction))
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("modification operation must use a positive Lamport timestamp")
        );
        let counter = mutable.crdt.as_counter().expect("expected a counter crdt");
        assert_eq!(counter.value(), 0);
        assert_eq!(mutable.op_id.lamport, 0);
    }
}
