use std::sync::Arc;

use parking_lot::RwLock;
use tracing::{error, instrument};

#[cfg(test)]
use crate::datatypes::wired_interceptor::WiredInterceptor;
use crate::{
    DatatypeState,
    datatypes::{
        common::Attribute, mutable::MutableDatatype, pull_handler::PullHandler,
        push_buffer::PushBuffer,
    },
    defaults,
    errors::push_pull::ClientPushPullError,
    observability::macros::add_span_event,
    types::{push_pull_pack::PushPullPack, uid::Cuid},
};

pub struct WiredDatatype {
    pub mutable: Arc<RwLock<MutableDatatype>>,
    pub attr: Arc<Attribute>,
    #[cfg(test)]
    interceptor: Arc<WiredInterceptor>,
}

impl WiredDatatype {
    pub fn new(mutable: Arc<RwLock<MutableDatatype>>, attr: Arc<Attribute>) -> Self {
        Self {
            mutable,
            attr,
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
            mutable: Arc::new(RwLock::new(MutableDatatype::new(attr.clone(), state))),
            attr,
            interceptor,
        })
    }

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
        #[cfg_attr(not(test), allow(unused_mut))]
        let mut pushing_ppp = mutable.create_push_pull_pack()?;

        #[cfg(test)]
        self.interceptor.before_push(&mut pushing_ppp);

        add_span_event!("send PUSH PushPullPack", "ppp"=> pushing_ppp.to_string());
        #[cfg_attr(not(test), allow(unused_mut))]
        let mut pulled_ppp = connectivity.push_and_pull(&pushing_ppp)?;

        #[cfg(test)]
        self.interceptor.after_pull(&mut pulled_ppp)?;

        add_span_event!("recv PULL PushPullPack", "ppp"=> pulled_ppp.to_string());

        let mut pull_handler = PullHandler::new(&mut pulled_ppp, &mut mutable);
        pull_handler.apply()
    }

    pub fn cuid(&self) -> Cuid {
        self.attr.cuid()
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
