use std::sync::Arc;

use parking_lot::RwLock;
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
    errors::datatypes::{DatatypeAction, DatatypeErrorWithActions},
    observability::{metrics, trace::add_span_event},
    operations::transaction::Transaction,
    types::{notification::Notification, push_pull_pack::PushPullPack, uid::Cuid},
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
            mutable: Arc::new(RwLock::new(MutableDatatype::new(
                attr.clone(),
                state,
                Default::default(),
            ))),
            attr,
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

    pub fn handle_error(&self, err: DatatypeError, action: DatatypeAction) {
        match action {
            DatatypeAction::Normal => {}
            DatatypeAction::Restart => {
                let mut mutable = self.mutable.write();
                mutable.reset();
                mutable.set_state(DatatypeState::SubscribingOrCreating);
            }
            DatatypeAction::Disable => self.mutable.write().disable(),
            DatatypeAction::Rollback => {
                self.mutable.write().do_rollback();
            }
        }
        self.mutable.read().call_error_handler(err);
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
    pub fn push_pull(&self) -> Result<(), DatatypeErrorWithActions> {
        let start = std::time::Instant::now();
        let result = self.do_push_pull();
        metrics::emit_sync(&self.attr, result.is_ok(), start.elapsed());
        result
    }

    fn do_push_pull(&self) -> Result<(), DatatypeErrorWithActions> {
        let connectivity = &self.attr.client_common.connectivity;

        #[cfg_attr(not(test), allow(unused_mut))]
        let mut pushing_ppp = {
            let mut mutable = self.mutable.write();
            mutable.create_push_pull_pack().map_err(|e| e.mapping())?
        };

        #[cfg(test)]
        self.interceptor.before_push(&mut pushing_ppp);

        add_span_event!("send PUSH PushPullPack", "ppp"=> pushing_ppp.to_string());
        #[cfg_attr(not(test), allow(unused_mut))]
        let mut pulled_ppp = connectivity
            .push_pull(&pushing_ppp)
            .map_err(|e| e.to_datatype_error().mapping())?;

        #[cfg(test)]
        self.interceptor.after_pull(&mut pulled_ppp)?;

        add_span_event!("recv PULL PushPullPack", "ppp"=> pulled_ppp.to_string());

        let mut mutable = self.mutable.write();
        let mut pull_handler = PullHandler::new(&mut pulled_ppp, &mut mutable);
        pull_handler.apply()
    }

    pub fn cuid(&self) -> Cuid {
        self.attr.get_cuid()
    }

    pub fn get_subscribe_snapshot(&self) -> Transaction {
        let m = self.mutable.write();
        let snap_op = m.new_snapshot_operation();
        let mut tx = Transaction::new_with_cuid(&self.cuid());
        tx.push_operation(snap_op);
        tx.sseq = m.checkpoint.sseq;
        tx
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
    use crate::{
        DataType, DatatypeState,
        datatypes::{
            common::new_attribute, wired::WiredDatatype, wired_interceptor::WiredInterceptor,
        },
    };

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
}
