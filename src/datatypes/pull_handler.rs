use tracing::{debug, instrument};

use crate::{
    DatatypeError, DatatypeState, ServerRejectReason,
    datatypes::{mutable::MutableDatatype, push_buffer::PushBuffer},
    errors::datatypes::DatatypeErrorWithAction,
    observability::trace::add_span_event,
    types::{checkpoint::CheckPoint, push_pull_pack::PushPullPack},
};

type PendingStep<'b> = fn(&mut PullHandler<'b>) -> Result<(), DatatypeErrorWithAction>;

pub struct PullHandler<'a> {
    pulled_ppp: &'a mut PushPullPack,
    mutable: &'a mut MutableDatatype,
    old_state: DatatypeState,
    new_state: DatatypeState,
    is_created: bool,
    pending_steps: Vec<PendingStep<'a>>,
    skip: usize,
    sent_cseq: u64,
}

impl<'a> PullHandler<'a> {
    pub fn new(
        pulled_ppp: &'a mut PushPullPack,
        mutable: &'a mut MutableDatatype,
        sent_cseq: u64,
    ) -> Self {
        let old_state = mutable.get_state();
        Self {
            pulled_ppp,
            mutable,
            old_state,
            new_state: old_state,
            is_created: false,
            pending_steps: Vec::new(),
            skip: 0,
            sent_cseq,
        }
    }

    #[instrument(skip_all, name = "applyPull")]
    pub fn apply(&mut self) -> Result<(), DatatypeErrorWithAction> {
        // Validate successful response progress before snapshot or CRDT mutations.
        // Backend errors follow their existing lifecycle and recovery policy.
        if self.pulled_ppp.error.is_none() {
            self.validate_checkpoint()?;
        }
        let result = (|| -> Result<(), DatatypeErrorWithAction> {
            self.handle_error_and_datatype_state()?;
            self.enqueue_step(Self::skip_duplicated_transactions);
            self.enqueue_step(Self::execute_transactions);
            self.enqueue_step(Self::sync_checkpoint);
            Ok(())
        })();
        self.commit()?;
        if result.is_ok() {
            // Reclaim only after every response step succeeds, under the same mutable
            // lock as checkpoint advancement. Failed application retains retry data.
            self.mutable.push_buffer.deque(self.mutable.checkpoint.cseq);
        }
        result
    }

    fn validate_checkpoint(&self) -> Result<(), DatatypeErrorWithAction> {
        let old = &self.mutable.checkpoint;
        let new = &self.pulled_ppp.checkpoint;
        if new.cseq > self.sent_cseq
            || new.cseq < old.cseq
            || new.sseq < old.sseq
            || new.sseq - old.sseq < new.cseq - old.cseq
        {
            return Err(
                DatatypeError::ServerRejected(ServerRejectReason::ProtocolViolation(format!(
                    "invalid response checkpoint {new}; previous {old}, sent cseq {}",
                    self.sent_cseq
                )))
                .mapping(),
            );
        }
        Ok(())
    }

    fn process_illegal_state_response(
        &self,
        old: DatatypeState,
        new: DatatypeState,
    ) -> Result<(), DatatypeErrorWithAction> {
        Err(
            DatatypeError::ServerRejected(ServerRejectReason::ProtocolViolation(format!(
                "illegal state from push-pull: received {new} for {old}"
            )))
            .mapping(),
        )
    }

    fn handle_error_and_datatype_state(&mut self) -> Result<(), DatatypeErrorWithAction> {
        self.new_state = self.pulled_ppp.state;
        if let Some(sppe) = self.pulled_ppp.error.as_ref() {
            return Err(sppe.to_datatype_error().mapping());
        }

        match self.old_state {
            DatatypeState::Creating => {
                if self.pulled_ppp.state != DatatypeState::Subscribed {
                    self.new_state = DatatypeState::Disabled;
                    self.process_illegal_state_response(self.old_state, self.pulled_ppp.state)?;
                }
                self.is_created = true;
            }
            DatatypeState::Subscribing => {
                if self.pulled_ppp.state != DatatypeState::Subscribed {
                    self.new_state = DatatypeState::Disabled;
                    self.process_illegal_state_response(self.old_state, self.pulled_ppp.state)?;
                }
                self.enqueue_step(Self::apply_subscribe_response);
            }
            DatatypeState::SubscribingOrCreating => {
                if self.pulled_ppp.state != DatatypeState::Subscribed {
                    self.new_state = DatatypeState::Disabled;
                    self.process_illegal_state_response(self.old_state, self.pulled_ppp.state)?;
                }
                if self.pulled_ppp.duid == self.mutable.attr.get_duid() {
                    self.is_created = true;
                } else {
                    self.enqueue_step(Self::apply_subscribe_response);
                }
            }
            DatatypeState::Subscribed => {
                if self.pulled_ppp.state != DatatypeState::Subscribed {
                    self.new_state = DatatypeState::Disabled;
                    self.process_illegal_state_response(self.old_state, self.pulled_ppp.state)?;
                }
            }
            DatatypeState::Unsubscribing => {
                if self.pulled_ppp.state != DatatypeState::Disabled {
                    self.new_state = DatatypeState::Disabled;
                    self.process_illegal_state_response(self.old_state, self.pulled_ppp.state)?;
                }
            }
            DatatypeState::Deleting => {
                todo!()
            }
            DatatypeState::Disabled => {
                todo!()
            }
        }

        if self.new_state != self.old_state {
            add_span_event!("changeState", "old" => format!("{}", self.old_state), "new" => format!("{}", self.new_state));
        }
        Ok(())
    }

    fn enqueue_step(&mut self, step: PendingStep<'a>) {
        self.pending_steps.push(step);
    }

    fn apply_subscribe_response(&mut self) -> Result<(), DatatypeErrorWithAction> {
        if let Some(snapshot_tx) = self.pulled_ppp.snapshot_transaction.take() {
            self.mutable
                .apply_snapshot_transaction(snapshot_tx)
                .map_err(|e| e.mapping())?;
        }
        self.mutable.attr.set_duid(self.pulled_ppp.duid.clone());
        Ok(())
    }

    fn skip_duplicated_transactions(&mut self) -> Result<(), DatatypeErrorWithAction> {
        let need_to_pull = self.calculate_pulling_transactions(&self.pulled_ppp.checkpoint);
        let len_pulled_txs = self.pulled_ppp.transactions.len();
        if len_pulled_txs > need_to_pull {
            self.skip = len_pulled_txs - need_to_pull;
            debug!("skip {} duplicated transactions", self.skip);
        }
        Ok(())
    }

    fn calculate_pulling_transactions(&self, new_cp: &CheckPoint) -> usize {
        let old_cp = &self.mutable.checkpoint;
        (new_cp.sseq - old_cp.sseq) as usize - (new_cp.cseq - old_cp.cseq) as usize
    }

    fn execute_transactions(&mut self) -> Result<(), DatatypeErrorWithAction> {
        let transactions = self.pulled_ppp.transactions[self.skip..].to_vec();
        for tx in transactions {
            self.mutable
                .execute_remote_transaction(tx)
                .map_err(|e| e.mapping())?;
        }
        Ok(())
    }

    fn sync_checkpoint(&mut self) -> Result<(), DatatypeErrorWithAction> {
        self.mutable
            .checkpoint
            .check_with(&self.pulled_ppp.checkpoint);
        Ok(())
    }

    fn commit(&mut self) -> Result<(), DatatypeErrorWithAction> {
        let steps = std::mem::take(&mut self.pending_steps);
        for step in steps {
            step(self)?;
        }
        self.mutable.set_state(self.new_state);
        Ok(())
    }
}

#[cfg(test)]
mod tests_push_handlers {
    use std::{sync::Arc, time::Duration};

    use rstest::rstest;
    use tracing::{info, instrument};

    use crate::{
        Client, DataType, Datatype, DatatypeState,
        datatypes::{
            common::new_attribute, mutable::MutableDatatype, pull_handler::PullHandler,
            push_buffer::PushBuffer,
        },
        operations::{MemoryMeasurable, Operation, transaction::Transaction},
        types::{checkpoint::CheckPoint, push_pull_pack::PushPullPack, uid::Cuid},
        utils::test_utils::{get_test_collection_name, get_test_func_name},
    };

    fn buffered_counter() -> MutableDatatype {
        let mut mutable = MutableDatatype::new(
            new_attribute!(DataType::Counter),
            DatatypeState::Subscribed,
            Default::default(),
        );
        for _ in 0..3 {
            mutable
                .execute_local_operation(Operation::new_counter_increase(1))
                .unwrap();
            mutable.end_transaction(None, true).unwrap();
        }
        mutable
    }

    fn response(mutable: &MutableDatatype, checkpoint: CheckPoint) -> PushPullPack {
        let mut response = PushPullPack::new(&mutable.attr, DatatypeState::Subscribed);
        response.checkpoint = checkpoint;
        response
    }

    #[test]
    fn can_reclaim_partial_and_duplicate_acknowledgements() {
        let mut mutable = buffered_counter();
        let weak: Vec<_> = mutable.push_buffer.iter().map(Arc::downgrade).collect();
        let tx_size = mutable.push_buffer.iter().next().unwrap().size();
        for acknowledged in [1, 1, 3, 3] {
            let mut response = response(&mutable, CheckPoint::new(acknowledged, acknowledged));
            PullHandler::new(&mut response, &mut mutable, 3)
                .apply()
                .unwrap();
            assert_eq!(
                mutable.checkpoint,
                CheckPoint::new(acknowledged, acknowledged)
            );
            assert_eq!(mutable.push_buffer.mem_size, (3 - acknowledged) * tx_size);
            assert_eq!(
                mutable.push_buffer.first_cseq,
                if acknowledged == 3 { 0 } else { 2 }
            );
            assert_eq!(mutable.push_buffer.last_cseq, 3);
            let (retry, _) = mutable
                .push_buffer
                .get_pushing_transactions(acknowledged + 1, u64::MAX)
                .unwrap();
            assert_eq!(
                retry.iter().map(|tx| tx.cseq).collect::<Vec<_>>(),
                ((acknowledged + 1)..=3).collect::<Vec<_>>()
            );
            drop(retry);
            for (i, tx) in weak.iter().enumerate() {
                assert_eq!(tx.upgrade().is_none(), i < acknowledged as usize);
            }
        }
        assert_eq!(mutable.crdt.as_counter().unwrap().value(), 3);
        assert_eq!(mutable.op_id.cseq, 3);
        mutable
            .execute_local_operation(Operation::new_counter_increase(1))
            .unwrap();
        mutable.end_transaction(None, true).unwrap();
        assert_eq!(mutable.push_buffer.first_cseq, 4);
        assert_eq!(mutable.push_buffer.last_cseq, 4);
    }

    #[rstest]
    #[case(CheckPoint::new(4, 4))]
    #[case(CheckPoint::new(0, 1))]
    #[case(CheckPoint::new(2, 0))]
    #[case(CheckPoint::new(1, 2))]
    fn can_reject_invalid_acknowledgements_without_reclaiming(#[case] checkpoint: CheckPoint) {
        let mut mutable = buffered_counter();
        let mut first = response(&mutable, CheckPoint::new(1, 1));
        PullHandler::new(&mut first, &mut mutable, 3)
            .apply()
            .unwrap();
        let memory = mutable.push_buffer.mem_size;
        let mut invalid = response(&mutable, checkpoint);
        assert!(
            PullHandler::new(&mut invalid, &mut mutable, 3)
                .apply()
                .is_err()
        );
        assert_eq!(mutable.checkpoint, CheckPoint::new(1, 1));
        assert_eq!(mutable.push_buffer.mem_size, memory);
        assert_eq!(
            mutable
                .push_buffer
                .iter()
                .map(|tx| tx.cseq)
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(mutable.crdt.as_counter().unwrap().value(), 3);
        assert_eq!(mutable.get_state(), DatatypeState::Subscribed);
    }

    #[test]
    fn can_keep_retry_data_when_response_application_fails() {
        let mut mutable = buffered_counter();
        let memory = mutable.push_buffer.mem_size;
        let mut failed = response(&mutable, CheckPoint::new(4, 3));
        let mut invalid = Transaction::new(&Cuid::new(), 1);
        invalid.sseq = 4;
        invalid.push_operation(Operation::new_snapshot(Vec::new().into_boxed_slice()));
        failed.transactions.push(Arc::new(invalid));
        assert!(
            PullHandler::new(&mut failed, &mut mutable, 3)
                .apply()
                .is_err()
        );
        assert_eq!(mutable.checkpoint, CheckPoint::default());
        assert_eq!(mutable.push_buffer.mem_size, memory);
        assert_eq!(mutable.push_buffer.iter().count(), 3);
        let (retry, _) = mutable
            .push_buffer
            .get_pushing_transactions(1, u64::MAX)
            .unwrap();
        assert_eq!(
            retry.iter().map(|tx| tx.cseq).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        let mut successful = response(&mutable, CheckPoint::new(3, 3));
        PullHandler::new(&mut successful, &mut mutable, 3)
            .apply()
            .unwrap();
        assert_eq!(mutable.checkpoint, CheckPoint::new(3, 3));
        assert_eq!(mutable.push_buffer.mem_size, 0);
        assert_eq!(mutable.push_buffer.iter().count(), 0);
        assert_eq!(mutable.crdt.as_counter().unwrap().value(), 3);
    }

    #[test]
    #[instrument]
    fn can_check_versions() {
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .build()
            .unwrap();
        let counter = client
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();
        assert_eq!(counter.get_client_version(), 0);
        assert_eq!(counter.get_server_version(), 0);
        counter.increase_by(1).unwrap();
        counter.increase().unwrap();
        awaitility::at_most(Duration::from_secs(1))
            .poll_interval(Duration::from_micros(100))
            .until(|| {
                counter.get_state() == DatatypeState::Subscribed
                    && counter.get_server_version() == 2
            });
        assert_eq!(counter.get_server_version(), counter.get_client_version());
        assert_eq!(
            counter.get_client_version(),
            counter.get_synced_client_version()
        );

        counter.increase_by(2).unwrap();
        counter.increase_by(3).unwrap();
        counter.sync().unwrap();

        awaitility::at_most(Duration::from_secs(1))
            .poll_interval(Duration::from_micros(100))
            .until(|| {
                let v = counter.get_server_version();
                info!("server version: {}", v);
                v == 4
            });
    }
}
