use std::sync::Arc;

use parking_lot::{Mutex, RwLock};
use tracing::{instrument, trace, warn};

#[cfg(test)]
use crate::datatypes::wired_interceptor::WiredInterceptor;
use crate::{
    DatatypeError, DatatypeState,
    datatypes::{
        common::Attribute, mutable::MutableDatatype, pull_handler::PullHandler,
        push_buffer::PushBuffer,
    },
    defaults,
    errors::datatypes::{DatatypeErrorWithAction, RecoveryAction},
    observability::{metrics, trace::add_span_event},
    operations::transaction::Transaction,
    types::{notification::Notification, push_pull_pack::PushPullPack, uid::Cuid},
};

pub struct WiredDatatype {
    pub mutable: Arc<RwLock<MutableDatatype>>,
    pub attr: Arc<Attribute>,
    // Shared with TransactionalDatatype so remote mutations cannot interleave
    // with a local transaction's commit or rollback.
    tx_mutex: Arc<Mutex<()>>,
    #[cfg(test)]
    interceptor: Arc<WiredInterceptor>,
}

impl WiredDatatype {
    pub fn new(
        mutable: Arc<RwLock<MutableDatatype>>,
        attr: Arc<Attribute>,
        tx_mutex: Arc<Mutex<()>>,
    ) -> Self {
        Self {
            mutable,
            attr,
            tx_mutex,
            #[cfg(test)]
            interceptor: WiredInterceptor::new_arc(),
        }
    }

    #[cfg(test)]
    pub fn new_arc_for_test(
        attr: Arc<Attribute>,
        state: DatatypeState,
        interceptor: Arc<WiredInterceptor>,
    ) -> Arc<Self> {
        Arc::new(Self {
            mutable: Arc::new(RwLock::new(MutableDatatype::new(
                attr.clone(),
                state,
                Default::default(),
            ))),
            attr,
            tx_mutex: Default::default(),
            interceptor,
        })
    }

    #[cfg(test)]
    pub fn get_wired_interceptor(&self) -> Arc<WiredInterceptor> {
        self.interceptor.clone()
    }

    pub fn push_if_needed(&self) -> bool {
        if !self.attr.client_common.connectivity.is_realtime() || !self.mutable.read().need_push() {
            return false;
        }
        true
    }

    pub fn handle_error(&self, err: DatatypeError, recovery: RecoveryAction) {
        self.with_transaction_gate(|mutable| mutable.apply_action(recovery));
        self.mutable.read().call_error_handler(err);
    }

    /// Runs a state operation under the transaction gate, then the mutable write lock.
    /// Both guards are released before returning to the caller.
    fn with_transaction_gate<R>(&self, action: impl FnOnce(&mut MutableDatatype) -> R) -> R {
        let _tx_guard = self.tx_mutex.lock();
        let mut mutable = self.mutable.write();
        action(&mut mutable)
    }

    #[instrument(skip_all)]
    pub fn handle_notification(&self, notify: Notification) -> bool {
        if self.attr.get_duid() != notify.duid {
            warn!(
                "ignore {notify}: different duid(expected={})",
                self.attr.get_duid()
            );
            return false;
        }
        if self.attr.get_cuid() == notify.cuid {
            trace!("ignore {notify}: self-notification");
            return false;
        }
        let cp_sseq = self.mutable.read().checkpoint.sseq;
        if cp_sseq >= notify.sseq {
            trace!(
                "ignore {notify} due to current sseq({cp_sseq}) >= notification.sseq({})",
                notify.sseq
            );
            return false;
        }
        trace!(
            "schedule push-pull due to current sseq({cp_sseq}) < notification.sseq({})",
            notify.sseq
        );
        true
    }

    #[instrument(skip_all)]
    pub fn push_pull(&self) -> Result<(), DatatypeErrorWithAction> {
        let start = std::time::Instant::now();
        let result = self.do_push_pull();
        metrics::emit_sync(&self.attr, result.is_ok(), start.elapsed());
        result
    }

    fn do_push_pull(&self) -> Result<(), DatatypeErrorWithAction> {
        let connectivity = &self.attr.client_common.connectivity;

        #[cfg_attr(not(test), allow(unused_mut))]
        let mut pushing_ppp = {
            let mut mutable = self.mutable.write();
            mutable.create_push_pull_pack().map_err(|e| e.mapping())?
        };

        #[cfg(test)]
        self.interceptor.before_push(&mut pushing_ppp);

        // Local writes can finish during network exchange. Only transactions included
        // in this request may be acknowledged by its response.
        let sent_cseq = pushing_ppp.checkpoint.cseq;

        add_span_event!("send PUSH PushPullPack", "ppp"=> pushing_ppp.to_string());
        metrics::emit_pushed_transactions(&self.attr, pushing_ppp.transactions.len());
        #[cfg_attr(not(test), allow(unused_mut))]
        let mut pulled_ppp = connectivity
            .push_pull(&pushing_ppp)
            .map_err(|e| e.to_datatype_error().mapping())?;
        metrics::emit_pulled_transactions(&self.attr, pulled_ppp.transactions.len());

        #[cfg(test)]
        self.interceptor.after_pull(&mut pulled_ppp)?;

        add_span_event!("recv PULL PushPullPack", "ppp"=> pulled_ppp.to_string());

        self.with_transaction_gate(|mutable| {
            let mut pull_handler = PullHandler::new(&mut pulled_ppp, mutable, sent_cseq);
            pull_handler.apply()
        })
    }

    pub fn cuid(&self) -> Cuid {
        self.attr.get_cuid()
    }

    pub fn get_subscribe_snapshot(&self) -> Transaction {
        self.with_transaction_gate(|mutable| {
            let snap_op = mutable.new_snapshot_operation();
            let mut tx = Transaction::new_with_cuid(&self.cuid());
            tx.push_operation(snap_op);
            tx.sseq = mutable.checkpoint.sseq;
            tx
        })
    }
}

impl MutableDatatype {
    #[instrument(skip_all)]
    fn create_push_pull_pack(&mut self) -> Result<PushPullPack, DatatypeError> {
        let mut ppp = PushPullPack::new(&self.attr, self.get_state());

        let (transactions, _tx_size) = self.push_buffer.get_pushing_transactions(
            self.checkpoint.cseq + 1,
            defaults::DEFAULT_MAX_TRANSMISSION_SIZE,
        )?;

        ppp.transactions = transactions;
        ppp.checkpointing(&self.checkpoint, 0);
        Ok(ppp)
    }

    fn need_push(&self) -> bool {
        let state = self.get_state();
        state == DatatypeState::Creating
            || state == DatatypeState::Subscribing
            || state == DatatypeState::SubscribingOrCreating
            || state == DatatypeState::Unsubscribing
            || self.push_buffer.last_cseq > self.checkpoint.cseq
    }
}

#[cfg(test)]
mod tests_wired {
    use std::{
        sync::{Arc, mpsc},
        thread,
        time::Duration,
    };

    use crate::{
        DataType, DatatypeError, DatatypeState,
        datatypes::{
            common::new_attribute, wired::WiredDatatype, wired_interceptor::WiredInterceptor,
        },
        operations::{Operation, transaction::Transaction},
        types::uid::Cuid,
    };

    #[rstest::rstest]
    #[case(false)]
    #[case(true)]
    fn can_preserve_writes_not_included_in_the_request(#[case] invalid_ack: bool) {
        let interceptor = WiredInterceptor::new_arc();
        let wired = WiredDatatype::new_arc_for_test(
            new_attribute!(DataType::Counter),
            DatatypeState::Subscribed,
            interceptor.clone(),
        );
        wired.with_transaction_gate(|mutable| {
            mutable
                .execute_local_operation(Operation::new_counter_increase(1))
                .unwrap();
            mutable.end_transaction(None, true).unwrap();
        });
        let requests = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let observed = requests.clone();
        interceptor.set_before_push(move |push| {
            observed.lock().push(
                push.transactions
                    .iter()
                    .map(|tx| tx.cseq)
                    .collect::<Vec<_>>(),
            );
        });
        let weak = Arc::downgrade(&wired);
        interceptor.set_after_pull(move |pull| {
            // This write finishes after the request was built and cannot be acknowledged
            // by that request's response, even though it is now locally committed.
            weak.upgrade().unwrap().with_transaction_gate(|mutable| {
                mutable
                    .execute_local_operation(Operation::new_counter_increase(1))
                    .unwrap();
                mutable.end_transaction(None, true).unwrap();
            });
            if invalid_ack {
                pull.checkpoint.cseq = 2;
                pull.checkpoint.sseq = 2;
            }
            Ok(())
        });
        let result = wired.push_pull();
        assert_eq!(result.is_err(), invalid_ack);
        {
            let mutable = wired.mutable.read();
            assert_eq!(mutable.op_id.cseq, 2);
            assert_eq!(mutable.crdt.as_counter().unwrap().value(), 2);
            assert_eq!(mutable.checkpoint.cseq, if invalid_ack { 0 } else { 1 });
            assert_eq!(
                mutable
                    .push_buffer
                    .iter()
                    .map(|tx| tx.cseq)
                    .collect::<Vec<_>>(),
                if invalid_ack { vec![1, 2] } else { vec![2] }
            );
        }
        interceptor.set_after_pull(|_| Ok(()));
        wired.push_pull().unwrap();
        assert_eq!(
            *requests.lock(),
            vec![vec![1], if invalid_ack { vec![1, 2] } else { vec![2] }]
        );
        let mutable = wired.mutable.read();
        assert_eq!(mutable.checkpoint.cseq, 2);
        assert_eq!(mutable.push_buffer.mem_size, 0);
        assert_eq!(mutable.crdt.as_counter().unwrap().value(), 2);
    }

    #[test]
    fn can_retry_after_a_response_is_lost_without_reclaiming_its_batch() {
        let interceptor = WiredInterceptor::new_arc();
        let wired = WiredDatatype::new_arc_for_test(
            new_attribute!(DataType::Counter),
            DatatypeState::Subscribed,
            interceptor.clone(),
        );
        wired.with_transaction_gate(|mutable| {
            mutable
                .execute_local_operation(Operation::new_counter_increase(1))
                .unwrap();
            mutable.end_transaction(None, true).unwrap();
        });
        let size = wired.mutable.read().push_buffer.mem_size;
        interceptor
            .set_after_pull(|_| Err(DatatypeError::SyncFailed("response lost".into()).mapping()));
        assert!(wired.push_pull().is_err());
        assert_eq!(wired.mutable.read().checkpoint.cseq, 0);
        assert_eq!(wired.mutable.read().push_buffer.mem_size, size);
        interceptor.set_before_push(|push| {
            assert_eq!(push.transactions.len(), 1);
            assert_eq!(push.transactions[0].cseq, 1);
        });
        interceptor.set_after_pull(|_| Ok(()));
        wired.push_pull().unwrap();
        assert_eq!(wired.mutable.read().checkpoint.cseq, 1);
        assert_eq!(wired.mutable.read().push_buffer.mem_size, 0);
        assert_eq!(wired.mutable.read().crdt.as_counter().unwrap().value(), 1);
    }

    #[test]
    fn can_push_unsubscribing() {
        let attr = new_attribute!(DataType::Counter);
        let wired = WiredDatatype::new_arc_for_test(
            attr,
            DatatypeState::Unsubscribing,
            WiredInterceptor::new_arc(),
        );

        assert!(wired.push_if_needed());
    }

    #[test]
    fn can_preserve_remote_variable_set_after_local_transaction_rollback() {
        let attr = new_attribute!(DataType::Variable);
        let interceptor = WiredInterceptor::new_arc();
        let (response_ready_tx, response_ready_rx) = mpsc::channel();
        interceptor.set_after_pull(move |pull| {
            let remote_cuid = Cuid::try_from("0000000000000001").unwrap();
            let mut operation =
                Operation::new_variable_set(b"\"remote\"".to_vec().into_boxed_slice());
            operation.set_lamport(2);
            let mut transaction = Transaction::new(&remote_cuid, 1);
            transaction.sseq = 1;
            transaction.push_operation(operation);
            pull.transactions = vec![Arc::new(transaction)];
            pull.checkpoint.sseq = 1;
            response_ready_tx.send(()).unwrap();
            Ok(())
        });
        let wired = WiredDatatype::new_arc_for_test(attr, DatatypeState::Creating, interceptor);
        let worker_wired = wired.clone();
        let local_transaction = wired.tx_mutex.lock();
        wired
            .mutable
            .write()
            .execute_local_operation(Operation::new_variable_set(
                b"\"local\"".to_vec().into_boxed_slice(),
            ))
            .unwrap();
        let (finished_tx, finished_rx) = mpsc::channel();

        let worker = thread::spawn(move || {
            finished_tx.send(worker_wired.push_pull()).unwrap();
        });
        response_ready_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the backend response must arrive while the local transaction is active");
        assert!(finished_rx.try_recv().is_err());
        wired.mutable.write().do_rollback();

        drop(local_transaction);
        finished_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("response application must resume after the local transaction finishes")
            .unwrap();
        worker.join().unwrap();

        let mutable = wired.mutable.read();
        assert_eq!(mutable.crdt.as_variable().unwrap().value(), b"\"remote\"");
        assert_eq!(mutable.op_id.cseq, 0);
        assert_eq!(mutable.op_id.lamport, 2);
        assert_eq!(mutable.checkpoint.cseq, 0);
        assert_eq!(mutable.checkpoint.sseq, 1);
        assert_eq!(mutable.get_state(), DatatypeState::Subscribed);
    }
}
