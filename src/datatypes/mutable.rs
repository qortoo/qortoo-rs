use std::{collections::BTreeMap, sync::Arc};

use tracing::instrument;

use crate::{
    DatatypeError, DatatypeHandler, DatatypeState, ServerRejectReason,
    datatypes::{
        common::{Attribute, ReturnType},
        crdts::{Crdt, LocalOperationOutcome},
        effects::{DatatypeEffect, DatatypeEffectBatch},
        handler::HandlersManager,
        push_buffer::{MemoryPushBuffer, PushBuffer},
        tx_record::TxRecord,
    },
    errors::{
        datatypes::{DatatypeErrorWithAction, RecoveryAction},
        messages::snapshot as msg,
        with_err_out,
    },
    operations::{Operation, body::OperationBody, transaction::Transaction},
    types::{
        checkpoint::CheckPoint, operation_context::OperationContext, operation_id::OperationId,
    },
};

#[derive(Debug)]
pub struct MutableDatatype {
    pub attr: Arc<Attribute>,
    pub crdt: Crdt,
    pub op_id: OperationId,
    pub push_buffer: MemoryPushBuffer,
    pub checkpoint: CheckPoint,
    /// Applied sseq recorded after a successful pull leaves no pending transactions;
    /// reported as sseq_gc_safe in the next push so older buffered writes cannot outlive GC.
    pub(super) sseq_gc_safe: u64,
    pub(super) sub_gen: u64,
    pub(super) needs_snapshot: bool,
    state: DatatypeState,
    tx_record: TxRecord,
    handlers_manager: HandlersManager,
    effects: DatatypeEffectBatch,
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
            sseq_gc_safe: 0,
            sub_gen: attr.client_common.next_sub_gen(),
            needs_snapshot: false,
            handlers_manager: HandlersManager::new(attr.clone(), handlers),
            effects: Default::default(),
            attr,
            crdt,
            state,
            op_id,
        }
    }

    #[cfg(test)]
    pub(super) fn is_transaction_record_empty(&self) -> bool {
        self.tx_record.pending.is_none() && self.tx_record.rollback_action_count() == 0
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
                ServerRejectReason::ProtocolViolation(msg::NO_SNAPSHOT_OPERATION.to_owned()),
            ));
        }
        let snap_op = &tx.operations[0];
        if let OperationBody::Snapshot(body) = &snap_op.body {
            self.crdt
                .deserialize(&body.data, self.attr.client_common.snapshot_decode_limits)?;
            self.op_id.cseq = 0;
            self.op_id.lamport = snap_op.lamport;
            self.reset();
        } else {
            return Err(DatatypeError::ServerRejected(
                ServerRejectReason::ProtocolViolation(msg::NO_SNAPSHOT_OPERATION.to_owned()),
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
    /// `Ok(false)` when rolled back or when no operation was recorded.
    /// Policy and enqueue failures retain `pending` so that
    /// `RecoveryAction::RollbackTransaction` can undo the transaction.
    pub fn end_transaction(
        &mut self,
        tag: Option<String>,
        committed: bool,
    ) -> Result<bool, DatatypeErrorWithAction> {
        if !committed {
            self.do_rollback();
            return Ok(false);
        }

        if self.tx_record.pending.is_some()
            && let Some(info) = self.attr.client_common.get_collection_info()
        {
            info.validate_state_change(
                self.tx_record.initial_state_bytes,
                self.crdt.logical_state_bytes(),
            )
            .map_err(|detail| {
                DatatypeErrorWithAction::new(
                    DatatypeError::CollectionPolicyViolation(detail),
                    RecoveryAction::RollbackTransaction,
                )
            })?;
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
            return Ok(true);
        }
        Ok(false)
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
                self.sub_gen = self.attr.client_common.next_sub_gen();
                self.checkpoint = CheckPoint::default();
                self.sseq_gc_safe = 0;
                self.needs_snapshot = true;
                self.reset();
                self.set_state(DatatypeState::SubscribingOrCreating);
            }
            RecoveryAction::Disable => self.disable(),
        }
    }

    pub fn execute_remote_transaction(&mut self, tx: &Transaction) -> Result<(), DatatypeError> {
        self.crdt
            .execute_remote_transaction(tx, &mut self.op_id.lamport)
    }

    #[instrument(skip_all)]
    pub fn execute_local_operation(
        &mut self,
        mut op: Operation,
    ) -> Result<ReturnType, DatatypeError> {
        if let Some(info) = self.attr.client_common.get_collection_info() {
            info.validate_write(&op.body)
                .map_err(DatatypeError::CollectionPolicyViolation)?;
        }
        let state_bytes = self.crdt.logical_state_bytes();
        op.set_lamport(self.op_id.lamport + 1);
        let outcome = {
            let context = OperationContext::try_new(&op, &self.op_id.cuid)?;
            self.crdt.execute_local_operation(&context)?
        };
        let (return_value, rollback_action) = match outcome {
            LocalOperationOutcome::Applied {
                return_value,
                rollback_action,
            } => (return_value, rollback_action),
            LocalOperationOutcome::NoOp { return_value } => return Ok(return_value),
        };
        let is_new_tx = self.tx_record.record_operation(
            &self.op_id,
            self.state,
            state_bytes,
            op,
            rollback_action,
        );
        self.op_id.next(is_new_tx);
        Ok(return_value)
    }

    pub fn set_handler(
        &mut self,
        priority: usize,
        handler: DatatypeHandler,
    ) -> Option<Arc<DatatypeHandler>> {
        self.handlers_manager.replace_handler(priority, handler)
    }

    pub fn unset_handler(&mut self, priority: usize) -> Option<Arc<DatatypeHandler>> {
        self.handlers_manager.remove_handler(priority)
    }

    pub fn get_state(&self) -> DatatypeState {
        self.state
    }

    pub fn set_state(&mut self, new_state: DatatypeState) {
        let old_state = self.state;
        if old_state != new_state {
            self.state = new_state;
            // Detach before scheduling callbacks, so a Disabled callback can build
            // a replacement without observing the old instance in the client table.
            if new_state == DatatypeState::Disabled {
                if let Some(target) = self.attr.datatype_target() {
                    self.effects.push(DatatypeEffect::Detach(target));
                }
            }
            if let Some(notification) = self
                .handlers_manager
                .prepare_state_change(old_state, new_state)
            {
                self.effects.push(DatatypeEffect::Notify(notification));
            }
        }
    }

    pub fn reserve_error_notification(&mut self, err: DatatypeError) {
        if let Some(notification) = self.handlers_manager.prepare_error(err) {
            self.effects.push(DatatypeEffect::Notify(notification));
        }
    }

    pub(crate) fn take_effects(&mut self) -> DatatypeEffectBatch {
        std::mem::take(&mut self.effects)
    }
}

#[cfg(test)]
mod tests_mutable_datatype {
    use std::sync::Arc;

    use tracing::instrument;

    use super::MutableDatatype;
    use crate::{
        DataType, DatatypeError, DatatypeState, SnapshotErrorReason,
        datatypes::{common::new_attribute, transactional::TransactionalDatatype},
        operations::{Operation, transaction::Transaction},
        types::uid::Cuid,
        utils::test_utils::snapshot_write_options,
    };

    #[test]
    fn can_leave_no_trace_for_missing_map_removes() {
        let mut mutable = MutableDatatype::new(
            new_attribute!(DataType::Map),
            Default::default(),
            Default::default(),
        );
        let initial_id = mutable.op_id.clone();
        mutable
            .execute_local_operation(Operation::new_map_remove("missing".into()))
            .unwrap();
        assert_eq!(mutable.op_id, initial_id);
        assert!(mutable.tx_record.pending.is_none());
        assert_eq!(mutable.tx_record.rollback_action_count(), 0);
        assert!(!mutable.end_transaction(None, true).unwrap());
        assert_eq!(mutable.push_buffer.iter().count(), 0);

        mutable
            .execute_local_operation(Operation::new_map_put(
                "key".into(),
                Box::from(b"null".as_slice()),
            ))
            .unwrap();
        let pending = mutable.tx_record.pending.clone();
        let id = mutable.op_id.clone();
        mutable
            .execute_local_operation(Operation::new_map_remove("missing".into()))
            .unwrap();
        assert_eq!(mutable.op_id, id);
        assert_eq!(mutable.tx_record.pending, pending);
        assert_eq!(mutable.tx_record.rollback_action_count(), 1);
        assert!(mutable.end_transaction(None, true).unwrap());
        assert_eq!(mutable.push_buffer.iter().count(), 1);

        mutable
            .execute_local_operation(Operation::new_map_remove("key".into()))
            .unwrap();
        mutable.end_transaction(None, true).unwrap();
        let tombstone = mutable.crdt.serialize(snapshot_write_options()).unwrap();
        let id = mutable.op_id.clone();
        mutable
            .execute_local_operation(Operation::new_map_remove("key".into()))
            .unwrap();
        assert_eq!(mutable.op_id, id);
        assert_eq!(
            mutable.crdt.serialize(snapshot_write_options()).unwrap(),
            tombstone
        );
        assert!(!mutable.end_transaction(None, true).unwrap());
    }

    #[test]
    fn can_preserve_pending_map_work_when_an_operation_is_unsupported() {
        let mut mutable = MutableDatatype::new(
            new_attribute!(DataType::Map),
            Default::default(),
            Default::default(),
        );
        mutable
            .execute_local_operation(Operation::new_map_put(
                "key".into(),
                Box::from(b"1".as_slice()),
            ))
            .unwrap();
        let snapshot = mutable.crdt.serialize(snapshot_write_options()).unwrap();
        let pending = mutable.tx_record.pending.clone();
        let id = mutable.op_id.clone();
        assert!(
            mutable
                .execute_local_operation(Operation::new_counter_increase(1))
                .is_err()
        );
        assert_eq!(
            mutable.crdt.serialize(snapshot_write_options()).unwrap(),
            snapshot
        );
        assert_eq!(mutable.op_id, id);
        assert_eq!(mutable.tx_record.pending, pending);
        assert_eq!(mutable.tx_record.rollback_action_count(), 1);
        assert!(mutable.end_transaction(None, true).unwrap());
        assert_eq!(
            mutable.push_buffer.iter().next().unwrap().operations.len(),
            1
        );
    }

    #[test]
    fn can_observe_map_timestamps_only_after_successful_execution() {
        let mut mutable = MutableDatatype::new(
            new_attribute!(DataType::Map),
            Default::default(),
            Default::default(),
        );
        let cuid = Cuid::new();
        for (lamport, mut op, accepted) in [
            (
                10,
                Operation::new_map_put("key".into(), Box::from(b"1".as_slice())),
                true,
            ),
            (
                5,
                Operation::new_map_put("key".into(), Box::from(b"2".as_slice())),
                true,
            ),
            // The backend validates JSON. A stale Put needs only the LWW check here.
            (
                4,
                Operation::new_map_put("key".into(), Box::from(b"{".as_slice())),
                true,
            ),
            (
                10,
                Operation::new_map_put("key".into(), Box::from(b"3".as_slice())),
                false,
            ),
            (20, Operation::new_counter_increase(1), false),
        ] {
            op.set_lamport(lamport);
            let mut tx = Transaction::new(&cuid, 1);
            tx.push_operation(op);
            let result = mutable.execute_remote_transaction(&tx);
            if accepted {
                result.unwrap();
            } else {
                assert!(matches!(result, Err(DatatypeError::Internal(..))));
            }
            assert_eq!(mutable.op_id.lamport, 10);
            assert_eq!(
                mutable
                    .crdt
                    .as_map()
                    .unwrap()
                    .shared_value("key")
                    .unwrap()
                    .as_ref(),
                b"1"
            );
        }
    }

    #[test]
    fn can_share_generations_across_datatypes_and_recovery() {
        use std::sync::Arc;

        use crate::{datatypes::common::Attribute, errors::datatypes::RecoveryAction};

        let first_attr = new_attribute!(DataType::Counter);
        let second_attr = Arc::new(Attribute::new(
            "another".into(),
            DataType::Counter,
            first_attr.client_common.clone(),
            Default::default(),
            false,
        ));
        let mut first =
            MutableDatatype::new(first_attr.clone(), Default::default(), Default::default());
        let first_generation = first.sub_gen;
        let second = MutableDatatype::new(second_attr, Default::default(), Default::default());
        assert!(first_generation < second.sub_gen);
        first.apply_action(RecoveryAction::RetryWithBackOff);
        assert_eq!(first.sub_gen, first_generation);
        first.apply_action(RecoveryAction::Resubscribe);
        assert!(second.sub_gen < first.sub_gen);
        let replacement = MutableDatatype::new(first_attr, Default::default(), Default::default());
        assert!(first.sub_gen < replacement.sub_gen);
    }

    #[test]
    #[instrument]
    fn can_preserve_transaction_state_when_operation_execution_fails() {
        let attr = new_attribute!(DataType::Counter);
        let tx_dt = TransactionalDatatype::new_arc(attr, Default::default(), Default::default());
        {
            let mutable = tx_dt.core.mutable.write();
            assert_eq!(0, mutable.op_id.cseq);
            assert!(mutable.tx_record.pending.is_none());
            assert_eq!(mutable.tx_record.rollback_action_count(), 0);
            assert_eq!(mutable.op_id, mutable.tx_record.rollback_op_id);
        }

        let op1 = Operation::new_counter_increase(1);
        let result1 = tx_dt.execute_local_operation_as_tx(Default::default(), op1);
        assert!(result1.is_ok());
        {
            let mutable = tx_dt.core.mutable.write();
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
            let mutable = tx_dt.core.mutable.write();
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
        let mut mutable = MutableDatatype::new(attr, Default::default(), Default::default());

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
        let mut mutable = MutableDatatype::new(attr, Default::default(), Default::default());
        let cuid = Cuid::try_from("0000000000000001").unwrap();
        let mut transaction = Transaction::new(&cuid, 1);
        let mut valid = Operation::new_counter_increase(3);
        valid.set_lamport(5);
        transaction.push_operation(valid);
        transaction.push_operation(Operation::new_counter_increase(1));

        let error = mutable
            .execute_remote_transaction(&transaction)
            .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("modification operation must use a positive Lamport timestamp")
        );
        let counter = mutable.crdt.as_counter().expect("expected a counter crdt");
        assert_eq!(counter.value(), 3);
        assert_eq!(mutable.op_id.lamport, 5);
    }

    #[test]
    fn can_preserve_pending_work_until_failed_snapshot_recovery_disables_it() {
        let mut mutable = MutableDatatype::new(
            new_attribute!(DataType::Counter),
            Default::default(),
            Default::default(),
        );
        mutable
            .execute_local_operation(Operation::new_counter_increase(3))
            .unwrap();
        mutable.end_transaction(None, true).unwrap();
        mutable
            .execute_local_operation(Operation::new_counter_increase(2))
            .unwrap();
        let previous_id = mutable.op_id.clone();
        let previous_checkpoint = mutable.checkpoint;
        let mut snapshot = Transaction::new(&Cuid::new_nil(), 0);
        let mut operation = Operation::new_snapshot(Box::from(b"invalid".as_slice()));
        operation.set_lamport(99);
        snapshot.push_operation(operation);
        let error = mutable
            .apply_snapshot_transaction(Arc::new(snapshot))
            .unwrap_err();
        assert!(matches!(
            &error,
            DatatypeError::SnapshotDecode(SnapshotErrorReason::Invalid)
        ));
        assert_eq!(mutable.crdt.as_counter().unwrap().value(), 5);
        assert_eq!(mutable.op_id, previous_id);
        assert_eq!(mutable.checkpoint, previous_checkpoint);
        assert!(mutable.tx_record.pending.is_some());
        assert_eq!(mutable.tx_record.rollback_action_count(), 1);
        assert!(mutable.push_buffer.iter().next().is_some());
        mutable.apply_action(error.mapping().recovery);
        assert_eq!(mutable.get_state(), DatatypeState::Disabled);
        assert!(mutable.is_transaction_record_empty());
        assert!(mutable.push_buffer.iter().next().is_none());
        assert_eq!(mutable.crdt.as_counter().unwrap().value(), 5);
        assert_eq!(mutable.op_id, previous_id);
    }
}
