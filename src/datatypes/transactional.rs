#[cfg(test)]
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{
    collections::BTreeMap,
    sync::Arc,
    thread::{self, ThreadId},
};

use parking_lot::{Mutex, MutexGuard, RwLock};
use tracing::{Span, info_span, instrument};

use crate::{
    DataType, DatatypeHandler, DatatypeState, IntoString,
    datatypes::{
        common::{Attribute, ReturnType, internal_datatype_instrument},
        datatype::Datatype,
        event_loop::EventLoop,
        mutable::MutableDatatype,
        wired::WiredDatatype,
    },
    errors::{
        datatypes::{DatatypeError, RecoveryAction},
        with_err_out,
    },
    observability::trace::add_span_event,
    operations::Operation,
};

#[derive(Debug, Default)]
pub struct TransactionContext {
    tag: Option<String>,
}

impl TransactionContext {
    pub fn new(tag: impl IntoString) -> Self {
        Self {
            tag: Some(tag.into()),
        }
    }

    pub fn has_tag(&self) -> bool {
        self.tag.is_some()
    }
}

impl PartialEq for TransactionContext {
    fn eq(&self, other: &Self) -> bool {
        let s_ptr: *const Self = self;
        let o_ptr: *const Self = other;
        s_ptr == o_ptr
    }
}

#[cfg(test)]
mod tests_transaction_context {
    use std::sync::Arc;

    use super::*;
    #[test]
    fn can_compare_transaction_contexts() {
        let ctx1 = TransactionContext::default();
        let ctx2 = TransactionContext::default();
        assert_eq!(ctx1, ctx1);
        assert_ne!(ctx1, ctx2);

        let ctx3 = Arc::new(TransactionContext::default());
        let ctx4 = ctx3.clone();
        assert_eq!(ctx3, ctx4);
        assert_ne!(ctx1, *ctx3);
    }
}

#[derive(derive_more::Display)]
enum TransactionAdmission<'a> {
    #[display("Started")]
    Started(TransactionGuard<'a>),
    #[display("JoinedExisting")]
    JoinedExisting,
}

/// Owns one local transaction and the mutex guard that admits it.
/// Joined operations and waiting callers never own this transaction's cleanup.
struct TransactionGuard<'a> {
    datatype: &'a TransactionalDatatype,
    context: Arc<TransactionContext>,
    mutex_guard: Option<MutexGuard<'a, ()>>,
    committed: bool,
}

impl TransactionGuard<'_> {
    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for TransactionGuard<'_> {
    fn drop(&mut self) {
        self.datatype
            .end_transaction(self.context.tag.clone(), self.committed);
        // Cleanup clears the context before fair handoff. If cleanup unwinds,
        // the remaining MutexGuard field still releases the mutex.
        if let Some(guard) = self.mutex_guard.take() {
            MutexGuard::unlock_fair(guard);
        }
    }
}

/// Associates the active context with the thread executing its local transaction.
struct ActiveTransactionContext {
    context: Arc<TransactionContext>,
    owner_thread: ThreadId,
}

pub struct TransactionalDatatype {
    pub attr: Arc<Attribute>,
    pub mutable: Arc<RwLock<MutableDatatype>>,
    active_tx_ctx: RwLock<Option<ActiveTransactionContext>>,
    op_mutex: Mutex<()>,
    tx_mutex: Arc<Mutex<()>>,
    #[cfg(test)]
    tx_waiters: AtomicUsize,
    event_loop: Arc<EventLoop>,
}

impl Datatype for TransactionalDatatype {
    fn get_key(&self) -> &str {
        self.attr.key.as_ref()
    }

    fn get_type(&self) -> DataType {
        self.attr.r#type
    }

    fn get_state(&self) -> DatatypeState {
        self.mutable.read().get_state()
    }

    fn get_server_version(&self) -> u64 {
        self.mutable.read().checkpoint.sseq
    }

    fn get_client_version(&self) -> u64 {
        self.mutable.read().op_id.cseq
    }

    fn get_synced_client_version(&self) -> u64 {
        self.mutable.read().checkpoint.cseq
    }

    fn sync(&self) -> Result<(), DatatypeError> {
        self.check_sync_allowed()?;
        self.event_loop.send_push_transaction_with_guarantee()
    }

    fn unsubscribe(&self) -> Result<(), DatatypeError> {
        self.check_subscribed("unsubscribe")?;
        self.mutable.write().set_state(DatatypeState::Unsubscribing);
        self.event_loop.send_push_transaction_with_best_effort();
        Ok(())
    }

    fn set_handler(&self, id: usize, handler: DatatypeHandler) {
        self.mutable.write().set_handler(id, handler)
    }

    fn unset_handler(&self, id: usize) -> bool {
        self.mutable.write().unset_handler(id)
    }

    #[cfg(test)]
    fn get_attr(&self) -> std::sync::Arc<Attribute> {
        self.attr.clone()
    }
}

impl TransactionalDatatype {
    pub fn new_arc(
        attr: Arc<Attribute>,
        state: DatatypeState,
        handlers: BTreeMap<usize, DatatypeHandler>,
    ) -> Arc<Self> {
        let event_loop = EventLoop::new_arc(attr.client_common.connectivity.clone());
        let arc_td = Arc::new(Self {
            mutable: Arc::new(RwLock::new(MutableDatatype::new(
                attr.clone(),
                state,
                handlers,
            ))),
            attr: attr.clone(),
            event_loop: event_loop.clone(),
            active_tx_ctx: Default::default(),
            op_mutex: Default::default(),
            tx_mutex: Default::default(),
            #[cfg(test)]
            tx_waiters: Default::default(),
        });

        attr.set_transactional(Arc::downgrade(&arc_td));
        let span = Span::current();
        span.in_scope(|| {
            event_loop.run(arc_td.get_wired_datatype());
        });
        arc_td
    }

    fn get_wired_datatype(&self) -> Arc<WiredDatatype> {
        Arc::new(WiredDatatype::new(
            self.mutable.clone(),
            self.attr.clone(),
            self.tx_mutex.clone(),
        ))
    }

    /// Checks whether this datatype is writable based on its readonly flag and state.
    ///
    /// Returns an error if:
    /// - The datatype is marked as readonly
    /// - The datatype state does not allow writes
    pub fn check_writable(&self) -> Result<(), DatatypeError> {
        use crate::errors::with_err_out;

        let state = self.get_state();
        if !state.is_read_writable() {
            return Err(with_err_out!(DatatypeError::NotWritable(format!(
                "Datatype '{}' cannot be modified for {:?} state",
                self.attr.key, state
            ))));
        }

        if self.attr.is_readonly {
            return Err(with_err_out!(DatatypeError::ReadonlyViolation));
        }

        Ok(())
    }

    fn check_sync_allowed(&self) -> Result<(), DatatypeError> {
        if self.mutable.read().get_state() == DatatypeState::Disabled {
            return Err(DatatypeError::NotWritable(
                "sync is not allowed in Disabled state".to_owned(),
            ));
        }
        // Response application uses the local transaction gate. Waiting for a
        // synchronous response from inside that transaction would form a cycle.
        if self
            .active_tx_ctx
            .read()
            .as_ref()
            .is_some_and(|active| active.owner_thread == thread::current().id())
        {
            return Err(DatatypeError::Disallowed(
                "sync is not allowed on the thread that owns the active local transaction"
                    .to_owned(),
            ));
        }
        Ok(())
    }

    fn check_subscribed(&self, op: &str) -> Result<(), DatatypeError> {
        let state = self.mutable.read().get_state();
        if state != DatatypeState::Subscribed {
            return Err(with_err_out!(DatatypeError::NotWritable(format!(
                "{} is only allowed in Subscribed state, current state is {:?}",
                op, state
            ))));
        }
        Ok(())
    }

    pub fn execute_local_operation_as_tx(
        &self,
        tx_ctx: Arc<TransactionContext>,
        op: Operation,
    ) -> Result<ReturnType, DatatypeError> {
        self.check_writable()?;
        self.with_local_transaction(self.begin_transaction(tx_ctx), || {
            let operation_guard = self.op_mutex.lock();
            let result = self.mutable.write().execute_local_operation(op);
            // Release mutable and the operation mutex before transaction cleanup takes
            // mutable again. On panic, stack unwinding preserves the same order.
            MutexGuard::unlock_fair(operation_guard);
            result
        })
    }

    /// Runs local work and marks an owned transaction for commit only on success.
    /// Joined work leaves commit and rollback to its enclosing transaction.
    fn with_local_transaction<R>(
        &self,
        admission: TransactionAdmission<'_>,
        action: impl FnOnce() -> Result<R, DatatypeError>,
    ) -> Result<R, DatatypeError> {
        match admission {
            TransactionAdmission::Started(mut guard) => action().inspect(|_| guard.commit()),
            TransactionAdmission::JoinedExisting => action(),
        }
    }

    #[instrument(skip_all)]
    fn end_transaction(&self, tag: Option<String>, committed: bool) {
        let error = {
            let mut mutable = self.mutable.write();
            match mutable.end_transaction(tag, committed) {
                Ok(true) => {
                    self.event_loop.send_push_transaction_with_best_effort();
                    None
                }
                Ok(false) => None,
                Err(dewa) => {
                    // Transaction cleanup runs before the API returns, but after its Result
                    // has been selected. This path does not go through the event loop:
                    // only RollbackTransaction is expected, and on_error reports it.
                    debug_assert!(matches!(dewa.recovery, RecoveryAction::RollbackTransaction));
                    mutable.apply_action(dewa.recovery);
                    Some(dewa.error)
                }
            }
        };
        if let Some(err) = error {
            // Notify after releasing the write lock; handlers may re-enter the datatype.
            self.mutable.read().call_error_handler(err);
        }
        self.active_tx_ctx.write().take();
    }

    fn begin_transaction(&self, tx_ctx: Arc<TransactionContext>) -> TransactionAdmission<'_> {
        // Only tagged contexts can join an explicit transaction. Standalone
        // handles share a default context but must always open distinct local transactions.
        if tx_ctx.has_tag() {
            let active = self.active_tx_ctx.read();
            if active.as_ref().map(|active| &active.context) == Some(&tx_ctx) {
                let tag = tx_ctx.tag.clone().unwrap_or("NONE".to_owned());
                drop(active);
                add_span_event!("JoinedLocalTransaction", "tag" => tag);
                return TransactionAdmission::JoinedExisting;
            }
        }

        // The current local transaction keeps this mutex until cleanup is complete.
        // A waiter that receives the guard becomes the next owner directly,
        // rather than releasing it and racing through begin_transaction again.
        #[cfg(test)]
        self.tx_waiters.fetch_add(1, Ordering::SeqCst);
        let mutex_guard = self.tx_mutex.lock();
        #[cfg(test)]
        self.tx_waiters.fetch_sub(1, Ordering::SeqCst);

        let current = if tx_ctx.has_tag() {
            tx_ctx
        } else {
            Arc::new(TransactionContext::default())
        };
        let tag = current.tag.clone().unwrap_or("NONE".to_owned());
        let mut active = self.active_tx_ctx.write();
        debug_assert!(active.is_none());
        *active = Some(ActiveTransactionContext {
            context: current.clone(),
            owner_thread: thread::current().id(),
        });
        drop(active);
        add_span_event!("StartedLocalTransaction", "tag" => tag);
        TransactionAdmission::Started(TransactionGuard {
            datatype: self,
            context: current,
            mutex_guard: Some(mutex_guard),
            committed: false,
        })
    }

    pub fn do_transaction<F>(
        &self,
        tx_ctx: Arc<TransactionContext>,
        tx_func: F,
    ) -> Result<(), DatatypeError>
    where
        F: FnOnce() -> Result<(), DatatypeError>,
    {
        let admission = self.begin_transaction(tx_ctx);
        if matches!(admission, TransactionAdmission::JoinedExisting) {
            // Public transaction entry points always create a fresh context.
            unreachable!("do_transaction cannot join its own active context");
        }
        self.with_local_transaction(admission, || {
            let tx_func_span = info_span!("tx_func");
            tx_func_span.in_scope(tx_func)
        })
    }

    pub fn push_transaction(&self) {
        // TODO: implement pushing transactions to remote server
    }
}

impl Drop for TransactionalDatatype {
    internal_datatype_instrument! {
    "drop_transactional_datatype",
    fn drop(&mut self) {
        self.event_loop.send_stop();
    }}
}

#[cfg(test)]
mod tests_transactional {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        sync::{Arc, atomic::Ordering, mpsc},
        thread,
        time::Duration,
    };

    use parking_lot::Mutex;
    use rstest::rstest;
    use tracing::{Span, info, info_span, instrument};
    use tracing_opentelemetry::OpenTelemetrySpanExt;

    use crate::{
        Client, DataType, Datatype, DatatypeError, DatatypeState, LocalConnectivity,
        datatypes::{
            common::new_attribute,
            option::DatatypeOption,
            transactional::{TransactionAdmission, TransactionContext, TransactionalDatatype},
        },
        operations::{MemoryMeasurable, Operation, transaction::Transaction},
        utils::test_utils::{get_test_collection_name, get_test_func_name},
    };

    const WAIT_TIMEOUT: Duration = Duration::from_secs(5);

    #[test]
    fn can_reject_sync_on_the_local_transaction_owner_thread() {
        let attr = new_attribute!(DataType::Counter);
        let datatype = TransactionalDatatype::new_arc(attr, Default::default(), Default::default());
        let TransactionAdmission::Started(_transaction) =
            datatype.begin_transaction(Arc::new(TransactionContext::new("active")))
        else {
            panic!("a new context must start a local transaction");
        };

        assert!(matches!(
            datatype.sync(),
            Err(DatatypeError::Disallowed(message))
                if message == "sync is not allowed on the thread that owns the active local transaction"
        ));
    }

    #[test]
    fn can_reject_sync_from_both_handles_inside_a_transaction_callback() {
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .build()
            .unwrap();
        let counter = client.create_datatype("counter").build_counter().unwrap();
        let original = counter.clone();
        counter
            .transaction("owner", move |transaction_handle| {
                assert!(matches!(
                    transaction_handle.sync(),
                    Err(DatatypeError::Disallowed(_))
                ));
                assert!(matches!(original.sync(), Err(DatatypeError::Disallowed(_))));
                transaction_handle.increase()?;
                Ok(())
            })
            .unwrap();
        counter.sync().unwrap();
        assert_eq!(counter.get_value(), 1);
    }

    #[test]
    fn can_complete_sync_from_another_thread_after_local_rollback() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let collection = get_test_collection_name!();
        let client = Client::builder(collection.clone(), get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter = client.create_datatype("counter").build_counter().unwrap();
        counter.sync().unwrap();
        let (response_tx, response_rx) = crossbeam_channel::unbounded();
        connectivity
            .get_wired_interceptor(&format!("{collection}/counter"), &client.get_cuid())
            .unwrap()
            .set_after_pull(move |_| {
                response_tx.send(()).unwrap();
                Ok(())
            });
        let (finished_tx, finished_rx) = crossbeam_channel::unbounded();
        let callback_finished_rx = finished_rx.clone();
        let sync_counter = counter.clone();
        let result = counter.transaction("rollback", move |transaction_handle| {
            transaction_handle.increase_by(10)?;
            thread::spawn(move || {
                finished_tx.send(sync_counter.sync()).unwrap();
            });
            response_rx.recv_timeout(WAIT_TIMEOUT).unwrap();
            // The network exchange finished while this callback still owns the gate.
            assert!(callback_finished_rx.try_recv().is_err());
            Err("rollback".into())
        });
        assert!(matches!(result, Err(DatatypeError::TransactionFailed(_))));
        finished_rx.recv_timeout(WAIT_TIMEOUT).unwrap().unwrap();
        assert_eq!(counter.get_value(), 0);
    }

    #[rstest]
    #[case(false)]
    #[case(true)]
    fn can_hold_standalone_transaction_until_commit_before_admitting_waiters(
        #[case] explicit_waiter: bool,
    ) {
        let attr = new_attribute!(DataType::Counter);
        let datatype = TransactionalDatatype::new_arc(attr, Default::default(), Default::default());
        let context = Arc::new(TransactionContext::default());
        let TransactionAdmission::Started(mut owner) = datatype.begin_transaction(context.clone())
        else {
            panic!("a standalone write must own a new local transaction");
        };

        // Pause the standalone path after its local transaction begins. Its mutex guard and
        // active context must already exclude competing local transactions.
        assert!(datatype.tx_mutex.try_lock().is_none());
        assert!(datatype.active_tx_ctx.read().is_some());

        let (waiting_tx, waiting_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let waiting_datatype = datatype.clone();
        let waiter = thread::spawn(move || {
            let context = Arc::new(if explicit_waiter {
                TransactionContext::new("waiting")
            } else {
                TransactionContext::default()
            });
            waiting_tx.send(()).unwrap();
            if explicit_waiter {
                waiting_datatype
                    .do_transaction(context.clone(), || {
                        waiting_datatype.execute_local_operation_as_tx(
                            context,
                            Operation::new_counter_increase(1),
                        )?;
                        Ok(())
                    })
                    .unwrap();
            } else {
                waiting_datatype
                    .execute_local_operation_as_tx(context, Operation::new_counter_increase(1))
                    .unwrap();
            }
            finished_tx.send(()).unwrap();
        });
        waiting_rx.recv_timeout(WAIT_TIMEOUT).unwrap();
        awaitility::at_most(WAIT_TIMEOUT).until(|| datatype.tx_waiters.load(Ordering::SeqCst) == 1);
        assert!(finished_rx.try_recv().is_err());

        datatype
            .mutable
            .write()
            .execute_local_operation(Operation::new_counter_increase(10))
            .unwrap();
        owner.commit();
        drop(owner);

        finished_rx.recv_timeout(WAIT_TIMEOUT).unwrap();
        waiter.join().unwrap();
        let mutable = datatype.mutable.read();
        assert_eq!(mutable.crdt.as_counter().unwrap().value(), 11);
        assert_eq!(mutable.op_id.cseq, 2);
        let values: Vec<_> = mutable
            .push_buffer
            .iter()
            .map(|tx| tx.operations.len())
            .collect();
        assert_eq!(values, vec![1, 1]);
        assert!(datatype.active_tx_ctx.read().is_none());
    }

    #[derive(Clone, Copy, Debug)]
    enum TransactionOutcome {
        Commit,
        CallbackError,
        Panic,
        EnqueueError,
    }

    #[rstest]
    #[case(TransactionOutcome::Commit)]
    #[case(TransactionOutcome::CallbackError)]
    #[case(TransactionOutcome::Panic)]
    #[case(TransactionOutcome::EnqueueError)]
    fn can_finish_transaction_cleanup_before_releasing_waiters(
        #[case] outcome: TransactionOutcome,
    ) {
        let attr = new_attribute!(DataType::Counter);
        let datatype = TransactionalDatatype::new_arc(attr, Default::default(), Default::default());
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (owner_done_tx, owner_done_rx) = mpsc::channel();
        let owner_datatype = datatype.clone();
        let owner = thread::spawn(move || {
            let context = Arc::new(TransactionContext::new("owner"));
            let result = catch_unwind(AssertUnwindSafe(|| {
                owner_datatype.do_transaction(context.clone(), || {
                    owner_datatype.execute_local_operation_as_tx(
                        context.clone(),
                        Operation::new_counter_increase(10),
                    )?;
                    // Joined operations release only their operation guards;
                    // the enclosing transaction retains its transaction guard.
                    assert!(owner_datatype.op_mutex.try_lock().is_some());
                    assert_eq!(
                        owner_datatype
                            .active_tx_ctx
                            .read()
                            .as_ref()
                            .map(|active| &active.context),
                        Some(&context)
                    );
                    assert!(owner_datatype.tx_mutex.try_lock().is_none());
                    if matches!(outcome, TransactionOutcome::EnqueueError) {
                        // Reject this two-operation transaction while leaving
                        // enough capacity for the waiter's single operation.
                        owner_datatype.execute_local_operation_as_tx(
                            context,
                            Operation::new_counter_increase(10),
                        )?;
                        let mut mutable = owner_datatype.mutable.write();
                        let op_id = &mutable.op_id;
                        let mut tx = Transaction::new(&op_id.cuid, 1);
                        tx.push_operation(Operation::new_counter_increase(1));
                        mutable.push_buffer.option = Arc::new(DatatypeOption {
                            max_mem_size_of_push_buffer: tx.size(),
                        });
                    }
                    entered_tx.send(()).unwrap();
                    release_rx.recv_timeout(WAIT_TIMEOUT).unwrap();
                    match outcome {
                        TransactionOutcome::Commit | TransactionOutcome::EnqueueError => Ok(()),
                        TransactionOutcome::CallbackError => {
                            Err(DatatypeError::TransactionFailed("abort".into()))
                        }
                        TransactionOutcome::Panic => panic!("abort the transaction by unwinding"),
                    }
                })
            }));
            match outcome {
                TransactionOutcome::Panic => assert!(result.is_err()),
                TransactionOutcome::CallbackError => assert!(result.unwrap().is_err()),
                // Enqueue rejection is reported through the error handler.
                TransactionOutcome::Commit | TransactionOutcome::EnqueueError => {
                    assert!(result.unwrap().is_ok())
                }
            }
            owner_done_tx.send(()).unwrap();
        });
        entered_rx.recv_timeout(WAIT_TIMEOUT).unwrap();

        let (waiting_tx, waiting_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let waiting_datatype = datatype.clone();
        let waiter = thread::spawn(move || {
            let context = Arc::new(TransactionContext::default());
            waiting_tx.send(()).unwrap();
            waiting_datatype
                .execute_local_operation_as_tx(context, Operation::new_counter_increase(1))
                .unwrap();
            finished_tx.send(()).unwrap();
        });
        waiting_rx.recv_timeout(WAIT_TIMEOUT).unwrap();
        awaitility::at_most(WAIT_TIMEOUT).until(|| datatype.tx_waiters.load(Ordering::SeqCst) == 1);
        assert!(finished_rx.try_recv().is_err());
        release_tx.send(()).unwrap();
        owner_done_rx.recv_timeout(WAIT_TIMEOUT).unwrap();
        finished_rx.recv_timeout(WAIT_TIMEOUT).unwrap();
        owner.join().unwrap();
        waiter.join().unwrap();

        let committed = matches!(outcome, TransactionOutcome::Commit);
        let mutable = datatype.mutable.read();
        assert_eq!(
            mutable.crdt.as_counter().unwrap().value(),
            if committed { 11 } else { 1 }
        );
        assert_eq!(mutable.op_id.cseq, if committed { 2 } else { 1 });
        assert_eq!(
            mutable.push_buffer.iter().count(),
            if committed { 2 } else { 1 }
        );
        assert!(datatype.active_tx_ctx.read().is_none());
        assert!(datatype.op_mutex.try_lock().is_some());
    }

    #[test]
    fn can_release_standalone_locks_after_operation_failure() {
        let attr = new_attribute!(DataType::Counter);
        let datatype = TransactionalDatatype::new_arc(attr, Default::default(), Default::default());
        assert!(
            datatype
                .execute_local_operation_as_tx(
                    Default::default(),
                    Operation::new_snapshot(Vec::new().into_boxed_slice()),
                )
                .is_err()
        );
        assert!(datatype.active_tx_ctx.read().is_none());
        assert!(datatype.op_mutex.try_lock().is_some());
        datatype
            .execute_local_operation_as_tx(Default::default(), Operation::new_counter_increase(1))
            .unwrap();
        assert_eq!(
            datatype.mutable.read().crdt.as_counter().unwrap().value(),
            1
        );
    }

    #[test]
    #[instrument]
    fn can_rollback_on_enqueue_failure() {
        let attr = new_attribute!(DataType::Counter);
        let tx_dt = TransactionalDatatype::new_arc(attr, Default::default(), Default::default());
        let tx_ctx = Arc::new(TransactionContext::new("non_sequential_cseq"));

        let tx_dt_in_tx = tx_dt.clone();
        let tx_ctx_in_tx = tx_ctx.clone();
        let result = tx_dt.do_transaction(tx_ctx, move || {
            tx_dt_in_tx.execute_local_operation_as_tx(
                tx_ctx_in_tx.clone(),
                Operation::new_counter_increase(5),
            )?;
            // Corrupt the buffer sequence so the commit-time enqueue fails with
            // InternalReason::NonSequentialCseq.
            tx_dt_in_tx.mutable.write().push_buffer.last_cseq = 42;
            Ok(())
        });
        // tx_func itself succeeded; the enqueue failure is routed to Normal + Rollback.
        assert!(result.is_ok());

        let mutable = tx_dt.mutable.read();
        let c = mutable.crdt.as_counter().expect("expected a counter crdt");
        assert_eq!(c.value(), 0);
        assert_eq!(mutable.op_id.cseq, 0);
        assert_ne!(mutable.get_state(), DatatypeState::Disabled);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 10)]
    #[instrument]
    async fn can_execute_the_same_tx_ctx_continuously_with_none_tx_ctx() {
        let client1 = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .build()
            .unwrap();
        let counter = client1
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();

        let counter_with_tx = counter.clone();
        let parent_span_no_tag = Span::current();
        let parent_span_tag = parent_span_no_tag.clone();
        let executions = Arc::new(Mutex::new(vec![]));
        let mut join_handles = vec![];

        for i in 0..5 {
            let parent_span_no_tag = parent_span_no_tag.clone();
            let counter_without_tx = counter.clone();
            let executions = executions.clone();
            join_handles.push(tokio::spawn(async move {
                let thread_span = info_span!("thread_with_no_tag", i = i);
                if !parent_span_no_tag.is_disabled() {
                    let _ = thread_span.set_parent(parent_span_no_tag.context());
                }
                let _g_thread_span = thread_span.enter();
                counter_without_tx.increase().unwrap();
                executions.lock().push(i + 1);
            }));
        }

        let executions_in_tx = executions.clone();
        join_handles.push(tokio::spawn(async move {
            let thread_span = info_span!("thread_with_tag");
            if !parent_span_tag.is_disabled() {
                let _ = thread_span.set_parent(parent_span_tag.context());
            }
            let _g_thread_span = thread_span.enter();
            counter_with_tx
                .transaction("with_tx", move |counter_in_tx| {
                    for i in 0..5 {
                        counter_in_tx.increase().unwrap();
                        executions_in_tx.lock().push(-(i + 1));
                    }
                    Ok(())
                })
                .unwrap();
        }));

        for jh in join_handles {
            jh.await.unwrap();
        }
        let executions = executions.lock();
        info!("{:?}", *executions);
        assert!(check_negatives_contiguous(&executions))
    }

    fn check_negatives_contiguous(nums: &Vec<i32>) -> bool {
        let mut in_negative_block = false;
        let mut finished_negative_block = false;

        for &n in nums {
            if n < 0 {
                if finished_negative_block {
                    return false;
                }
                in_negative_block = true;
            } else if in_negative_block {
                finished_negative_block = true;
                in_negative_block = false;
            }
        }
        true
    }
}
