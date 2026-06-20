use tracing::{debug, instrument};

use crate::{
    DatatypeError, DatatypeState,
    datatypes::mutable::MutableDatatype,
    errors::datatypes::{DatatypeAction, DatatypeErrorWithActions, EventLoopAction},
    observability::trace::add_span_event,
    types::{checkpoint::CheckPoint, push_pull_pack::PushPullPack},
};

type PendingStep<'b> = fn(&mut PullHandler<'b>) -> Result<(), DatatypeErrorWithActions>;

pub struct PullHandler<'a> {
    pulled_ppp: &'a mut PushPullPack,
    mutable: &'a mut MutableDatatype,
    old_state: DatatypeState,
    new_state: DatatypeState,
    is_created: bool,
    pending_steps: Vec<PendingStep<'a>>,
    skip: usize,
}

impl<'a> PullHandler<'a> {
    pub fn new(pulled_ppp: &'a mut PushPullPack, mutable: &'a mut MutableDatatype) -> Self {
        let old_state = mutable.get_state();
        Self {
            pulled_ppp,
            mutable,
            old_state,
            new_state: old_state,
            is_created: false,
            pending_steps: Vec::new(),
            skip: 0,
        }
    }

    #[instrument(skip_all, name = "applyPull")]
    pub fn apply(&mut self) -> Result<(), DatatypeErrorWithActions> {
        let result = (|| -> Result<(), DatatypeErrorWithActions> {
            self.handle_error_and_datatype_state()?;
            self.enqueue_step(Self::skip_duplicated_transactions);
            self.enqueue_step(Self::execute_transactions);
            self.enqueue_step(Self::sync_checkpoint);
            Ok(())
        })();
        self.commit()?;
        result
    }

    fn process_illegal_state_response(
        &self,
        old: DatatypeState,
        new: DatatypeState,
    ) -> Result<(), DatatypeErrorWithActions> {
        Err(DatatypeErrorWithActions::new(
            DatatypeError::FailedByProtocolViolation(format!(
                "illegal state from push-pull: received {new} for {old}"
            )),
            EventLoopAction::PauseSync,
            DatatypeAction::Disable,
        ))
    }

    fn handle_error_and_datatype_state(&mut self) -> Result<(), DatatypeErrorWithActions> {
        self.new_state = self.pulled_ppp.state;
        if let Some(sppe) = self.pulled_ppp.error.as_ref() {
            return Err(sppe.mapping(self.old_state, self.pulled_ppp.state));
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

    fn apply_subscribe_response(&mut self) -> Result<(), DatatypeErrorWithActions> {
        if let Some(snapshot_tx) = self.pulled_ppp.snapshot_transaction.take() {
            self.mutable
                .apply_snapshot_transaction(snapshot_tx)
                .map_err(|e| e.mapping())?;
        }
        self.mutable.attr.set_duid(self.pulled_ppp.duid.clone());
        Ok(())
    }

    fn skip_duplicated_transactions(&mut self) -> Result<(), DatatypeErrorWithActions> {
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

    fn execute_transactions(&mut self) -> Result<(), DatatypeErrorWithActions> {
        let transactions = self.pulled_ppp.transactions[self.skip..].to_vec();
        for tx in transactions {
            self.mutable.execute_remote_transaction(tx).map_err(|e| {
                DatatypeErrorWithActions::new(e, EventLoopAction::Normal, DatatypeAction::Restart)
            })?;
        }
        Ok(())
    }

    fn sync_checkpoint(&mut self) -> Result<(), DatatypeErrorWithActions> {
        self.mutable
            .checkpoint
            .check_with(&self.pulled_ppp.checkpoint);
        Ok(())
    }

    fn commit(&mut self) -> Result<(), DatatypeErrorWithActions> {
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
    use std::time::Duration;

    use tracing::{info, instrument};

    use crate::{
        Client, Datatype, DatatypeState,
        utils::test_utils::{get_test_collection_name, get_test_func_name},
    };

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
