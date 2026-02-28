use std::{collections::BTreeMap, sync::Arc};

use tracing::{Span, error, instrument};

use crate::{
    DatatypeError, DatatypeSet, DatatypeState, datatypes::common::Attribute,
    observability::macros::add_span_event,
};

/// Signature for a state-change handler.
///
/// Receives: `(datatype_set, old_state, new_state)`
pub type OnStateChangeFn = Box<dyn Fn(DatatypeSet, DatatypeState, DatatypeState) + Send + Sync>;

/// Signature for an error handler.
///
/// Receives: `(datatype_set, error)`
pub type OnErrorFn = Box<dyn Fn(DatatypeSet, &DatatypeError) + Send + Sync>;

/// Holds per-datatype event handlers for state changes and errors.
/// Default handlers are no-ops.
pub struct DatatypeHandler {
    on_state_change: OnStateChangeFn,
    on_error: OnErrorFn,
}

impl DatatypeHandler {
    pub fn new() -> Self {
        Self {
            on_state_change: Box::new(|_datatype_set, _old_state, _new_state| {}),
            on_error: Box::new(|_datatype_set, _err| {}),
        }
    }

    pub fn set_on_state_change(
        mut self,
        f: impl Fn(DatatypeSet, DatatypeState, DatatypeState) + Send + Sync + 'static,
    ) -> Self {
        self.on_state_change = Box::new(f);
        self
    }

    pub fn set_on_error(
        mut self,
        f: impl Fn(DatatypeSet, &DatatypeError) + Send + Sync + 'static,
    ) -> Self {
        self.on_error = Box::new(f);
        self
    }

    pub(crate) fn notify_state_change(
        &self,
        datatype_set: DatatypeSet,
        old_state: DatatypeState,
        new_state: DatatypeState,
    ) {
        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (self.on_state_change)(datatype_set, old_state, new_state)
        })) {
            error!("on_state_change handler panicked: {e:?}");
        }
    }

    #[allow(dead_code)]
    pub(crate) fn notify_error(&self, datatype_set: DatatypeSet, error: &DatatypeError) {
        if let Err(e) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            (self.on_error)(datatype_set, error)
        })) {
            error!("on_error handler panicked: {e:?}");
        }
    }
}

pub struct HandlersManager {
    handlers: BTreeMap<usize, Arc<DatatypeHandler>>,
    attr: Arc<Attribute>,
}

impl Default for DatatypeHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for HandlersManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HandlersManager")
            .field("handlers_count", &self.handlers.len())
            .finish()
    }
}

impl HandlersManager {
    pub fn new(attr: Arc<Attribute>, handlers: BTreeMap<usize, DatatypeHandler>) -> Self {
        Self {
            handlers: handlers
                .into_iter()
                .map(|(k, v)| (k, Arc::new(v)))
                .collect(),
            attr,
        }
    }

    pub fn set_handler(&mut self, priority: usize, handler: DatatypeHandler) {
        self.handlers.insert(priority, Arc::new(handler));
    }

    pub fn unset_handler(&mut self, priority: usize) -> Option<DatatypeHandler> {
        self.handlers
            .remove(&priority)
            .and_then(|arc| Arc::try_unwrap(arc).ok())
    }

    #[instrument]
    pub(crate) fn notify_state_change(&self, old_state: DatatypeState, new_state: DatatypeState) {
        let rt_handle = self.attr.client_common.handle.clone();
        if let Some(ds) = self.attr.get_datatype_set() {
            let handlers: Vec<(usize, Arc<DatatypeHandler>)> =
                self.handlers.iter().map(|(&k, v)| (k, v.clone())).collect();
            let span = Span::current();
            rt_handle.spawn(async move {
                for (priority, handler) in handlers {
                    span.in_scope(|| {
                        add_span_event!(format!("notify_state_change priority={priority}"));
                        handler.notify_state_change(ds.clone(), old_state, new_state);
                    });
                }
            });
        }
    }
}

#[cfg(test)]
mod tests_handers_manager {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use tracing::instrument;

    use crate::{
        Client, Datatype, DatatypeHandler, DatatypeState, LocalConnectivity,
        utils::path::{get_test_collection_name, get_test_func_name},
    };

    #[test]
    #[instrument]
    fn can_notify_state_change() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build()
            .unwrap();

        let call_count = Arc::new(AtomicUsize::new(0));
        let count_for_h1 = call_count.clone();
        let count_for_h2 = call_count.clone();

        let handler1 =
            DatatypeHandler::new().set_on_state_change(move |_ds, old_state, new_state| {
                let a = count_for_h1.fetch_add(1, Ordering::Relaxed);
                assert_eq!(a, 0);
                assert_eq!(old_state, DatatypeState::DueToCreate);
                assert_eq!(new_state, DatatypeState::Subscribed);
            });

        let handler2 =
            DatatypeHandler::new().set_on_state_change(move |_ds, old_state, new_state| {
                let a = count_for_h2.fetch_add(1, Ordering::Relaxed);
                assert_eq!(a, 1);
                assert_eq!(old_state, DatatypeState::DueToCreate);
                assert_eq!(new_state, DatatypeState::Subscribed);
            });

        let counter = client
            .create_datatype(get_test_func_name!())
            .with_handler(100, handler2)
            .build_counter()
            .unwrap();

        counter.set_handler(0, handler1);
        counter.sync().unwrap();
        awaitility::at_most(Duration::from_secs(2))
            .poll_interval(Duration::from_micros(100))
            .until(|| call_count.load(Ordering::Relaxed) == 2);
    }
}
