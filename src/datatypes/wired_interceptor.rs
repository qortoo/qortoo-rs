use std::sync::Arc;

use parking_lot::RwLock;

use crate::{errors::push_pull::ClientPushPullError, types::push_pull_pack::PushPullPack};

pub type BeforePushFn = Box<dyn Fn(&mut PushPullPack) + Send + Sync + 'static>;
pub type AfterPullFn =
    Box<dyn Fn(&mut PushPullPack) -> Result<(), ClientPushPullError> + Send + Sync + 'static>;

pub struct WiredInterceptor {
    before_push: RwLock<BeforePushFn>,
    after_pull: RwLock<AfterPullFn>,
}

impl WiredInterceptor {
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self {
            before_push: RwLock::new(Box::new(|_push| {})),
            after_pull: RwLock::new(Box::new(|_pull| Ok(()))),
        })
    }

    pub fn set_before_push(&self, f: impl Fn(&mut PushPullPack) + Send + Sync + 'static) -> &Self {
        *self.before_push.write() = Box::new(f);
        self
    }

    pub fn set_after_pull(
        &self,
        f: impl Fn(&mut PushPullPack) -> Result<(), ClientPushPullError> + Send + Sync + 'static,
    ) -> &Self {
        *self.after_pull.write() = Box::new(f);
        self
    }

    pub(crate) fn before_push(&self, push: &mut PushPullPack) {
        (self.before_push.read())(push)
    }

    pub(crate) fn after_pull(&self, pull: &mut PushPullPack) -> Result<(), ClientPushPullError> {
        (self.after_pull.read())(pull)
    }
}

#[cfg(test)]
mod tests_wired_interceptor {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use tracing::{info, instrument};

    use crate::{
        DataType, DatatypeState,
        datatypes::{
            common::new_attribute, wired::WiredDatatype, wired_interceptor::WiredInterceptor,
        },
    };

    #[test]
    #[instrument]
    fn can_use_wired_interceptor() {
        let attr = new_attribute!(DataType::Counter);

        let abool = Arc::new(AtomicBool::new(false));
        let abool_for_before_push = abool.clone();
        let abool_for_after_pull = abool.clone();
        let (tx, rx) = crossbeam_channel::unbounded();

        let wd_interceptor = WiredInterceptor::new_arc();
        wd_interceptor
            .set_before_push(move |push| {
                abool_for_before_push.store(true, Ordering::Relaxed);
                info!("INITIAL before_push: {push}");
            })
            .set_after_pull(move |pull| {
                assert!(abool_for_after_pull.load(Ordering::Relaxed));
                info!("INITIAL after_pull: {pull}");
                tx.send(()).unwrap();
                Ok(())
            });

        let wd = WiredDatatype::new_arc_for_test(
            attr,
            DatatypeState::DueToCreate,
            wd_interceptor.clone(),
        );

        let _ = wd.push_pull();
        let _ = rx.recv();

        let abool2 = Arc::new(AtomicBool::new(false));
        let abool2_for_before_push = abool2.clone();
        let abool2_for_after_pull = abool2.clone();
        let (tx, rx) = crossbeam_channel::unbounded();
        wd_interceptor
            .set_before_push(move |push| {
                abool2_for_before_push.store(true, Ordering::Relaxed);
                info!("MODIFIED before_push: {push}");
            })
            .set_after_pull(move |pull| {
                assert!(abool2_for_after_pull.load(Ordering::Relaxed));
                info!("MODIFIED after_pull: {pull}");
                tx.send(()).unwrap();
                Ok(())
            });

        let _ = wd.push_pull();
        let _ = rx.recv();
    }
}
