use tracing::instrument;

use crate::{
    DatatypeState,
    datatypes::mutable::MutableDatatype,
    errors::push_pull::{ClientPushPullError, ServerPushPullError},
    observability::macros::add_span_event,
    types::push_pull_pack::PushPullPack,
};

#[allow(dead_code)]
pub enum CaseAfterSync {
    Normal,
    BackOff,
    Reset,
    Halt,
}

pub struct PullHandler<'a> {
    pulled_ppp: &'a mut PushPullPack,
    mutable: &'a mut MutableDatatype,
    old_state: DatatypeState,
    new_state: DatatypeState,
    is_created: bool,
}

impl<'a> PullHandler<'a> {
    pub fn new(pulled_ppp: &'a mut PushPullPack, mutable: &'a mut MutableDatatype) -> Self {
        let old_state = mutable.state;
        Self {
            pulled_ppp,
            mutable,
            old_state,
            new_state: old_state,
            is_created: false,
        }
    }

    #[instrument(skip_all, name = "applyPull")]
    pub fn apply(&mut self) -> Result<(), ClientPushPullError> {
        self.handle_error_and_datatype_state()?;
        self.skip_duplicated_transactions()?;
        self.execute_transactions()?;
        self.sync_checkpoint()?;
        self.wrap_up()?;
        Ok(())
    }

    fn handle_error_and_datatype_state(&mut self) -> Result<(), ClientPushPullError> {
        if let Some(sppe) = self.pulled_ppp.error.as_ref() {
            match sppe {
                ServerPushPullError::IllegalPushRequest(reason) => {
                    // IllegalPushRequest indicates an unrecoverable state
                    return Err(ClientPushPullError::FailedAndAbort(reason.to_owned()));
                }
                ServerPushPullError::FailedToCreate(_err_msg) => {
                    // TODO: handle FailedToCreate
                }
                ServerPushPullError::FailedToSubscribe(_) => todo!(),
            }
        }

        match self.old_state {
            DatatypeState::DueToCreate => {
                if self.pulled_ppp.state == DatatypeState::DueToCreate {
                    self.new_state = DatatypeState::Subscribed;
                    self.is_created = true;
                } else {
                    // TODO: handle error
                }
            }
            DatatypeState::DueToSubscribe => {
                if self.pulled_ppp.state == DatatypeState::DueToSubscribe {
                    self.new_state = DatatypeState::Subscribed;
                } else {
                    // TODO: handle error
                }
            }
            DatatypeState::DueToSubscribeOrCreate => {
                if self.pulled_ppp.state == DatatypeState::DueToCreate {
                    self.new_state = DatatypeState::Subscribed;
                    self.is_created = true;
                } else if self.pulled_ppp.state == DatatypeState::DueToSubscribe {
                    self.new_state = DatatypeState::Subscribed;
                } else {
                    // TODO: handle error
                }
            }
            DatatypeState::Subscribed => {
                if self.pulled_ppp.state == DatatypeState::Subscribed {}
                // TODO: Handle Subscribed state
            }
            DatatypeState::DueToUnsubscribe => {
                // TODO: Handle DueToUnsubscribe state
            }
            DatatypeState::DueToDelete => {
                // TODO: Handle DueToDelete state
            }
            DatatypeState::Disabled => {
                // TODO: Handle Disabled state
            }
        }
        if self.new_state != self.old_state {
            add_span_event!("changeState", "old" => format!("{}", self.old_state), "new" => format!("{}", self.new_state));
        }
        Ok(())
    }

    fn skip_duplicated_transactions(&mut self) -> Result<(), ClientPushPullError> {
        // TODO: skip duplicated transactions
        Ok(())
    }

    fn execute_transactions(&mut self) -> Result<(), ClientPushPullError> {
        // TODO: execute transactions
        Ok(())
    }

    fn sync_checkpoint(&mut self) -> Result<(), ClientPushPullError> {
        self.mutable
            .checkpoint
            .check_with(&self.pulled_ppp.checkpoint);
        Ok(())
    }

    fn wrap_up(&mut self) -> Result<(), ClientPushPullError> {
        if self.old_state != self.new_state {
            self.mutable.state = self.new_state;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests_push_handlers {
    use std::time::Duration;

    use tracing::{info, instrument};

    use crate::{
        Client, Datatype, DatatypeState,
        utils::path::{get_test_collection_name, get_test_func_name},
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

        awaitility::at_most(Duration::from_secs(1))
            .poll_interval(Duration::from_micros(100))
            .until(|| {
                let v = counter.get_server_version();
                info!("server version: {}", v);
                v == 4
            });
    }
}
