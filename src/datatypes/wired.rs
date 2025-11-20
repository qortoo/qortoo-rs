use std::sync::Arc;

use parking_lot::RwLock;
use tracing::{error, instrument};

use crate::{
    DatatypeState,
    datatypes::{
        common::Attribute, mutable::MutableDatatype, pull_handler::PullHandler,
        push_buffer::PushBuffer,
    },
    defaults,
    errors::push_pull::ClientPushPullError,
    observability::macros::add_span_event,
    types::push_pull_pack::PushPullPack,
};

pub struct WiredDatatype {
    pub mutable: Arc<RwLock<MutableDatatype>>,
    pub attr: Arc<Attribute>,
}

impl WiredDatatype {
    pub fn push_if_needed(&self) {
        if !self.attr.client_common.connectivity.is_realtime() || !self.mutable.read().need_push() {
            return;
        }
        if let Err(e) = self.push_pull() {
            error!("push_pull failed: {}", e);
        }
    }

    #[instrument(skip_all)]
    pub fn push_pull(&self) -> Result<(), ClientPushPullError> {
        let connectivity = &self.attr.client_common.connectivity;

        let mut mutable = self.mutable.write();
        let pushing_ppp = mutable.create_push_pull_pack()?;

        add_span_event!("send PUSH PushPullPack", "ppp"=> pushing_ppp.to_string());
        let mut pulled_ppp = connectivity.push_and_pull(&pushing_ppp)?;
        add_span_event!("recv PULL PushPullPack", "ppp"=> pulled_ppp.to_string());

        let mut pull_handler = PullHandler::new(&mut pulled_ppp, &mut mutable);
        pull_handler.apply()
    }
}

impl MutableDatatype {
    #[instrument(skip_all)]
    fn create_push_pull_pack(&mut self) -> Result<PushPullPack, ClientPushPullError> {
        let mut ppp = PushPullPack::new(&self.attr, self.state);

        let (transactions, _tx_size) = self.push_buffer.get_after(
            self.checkpoint.cseq + 1,
            defaults::DEFAULT_MAX_TRANSMISSION_SIZE,
        )?;

        ppp.transactions = transactions;
        ppp.checkpointing(&self.checkpoint, 0);
        Ok(ppp)
    }

    fn need_push(&self) -> bool {
        self.state == DatatypeState::DueToCreate
            || self.state == DatatypeState::DueToSubscribe
            || self.state == DatatypeState::DueToSubscribeOrCreate
            || self.push_buffer.last_cseq > self.checkpoint.cseq
    }
}
