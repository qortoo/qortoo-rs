use std::{collections::BTreeMap, sync::Arc};

use tokio::runtime::Handle;
use tracing::{Span, error};

use crate::{
    DatatypeError, DatatypeSet, DatatypeState,
    datatypes::{common::Attribute, target::DatatypeTarget},
    observability::trace::add_span_event,
};

/// Signature for a state-change handler.
///
/// Receives: `(datatype_set, old_state, new_state)`
pub type OnStateChangeFn = Box<dyn Fn(DatatypeSet, DatatypeState, DatatypeState) + Send + Sync>;

/// Signature for an error handler.
///
/// Receives: `(datatype_set, error)`
pub type OnErrorFn = Box<dyn Fn(DatatypeSet, DatatypeError) + Send + Sync>;

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
        f: impl Fn(DatatypeSet, DatatypeError) + Send + Sync + 'static,
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

    pub(crate) fn notify_error(&self, datatype_set: DatatypeSet, error: DatatypeError) {
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

    /// Returns the previous registration so the mutable owner can drop it after unlocking.
    pub(crate) fn replace_handler(
        &mut self,
        priority: usize,
        handler: DatatypeHandler,
    ) -> Option<Arc<DatatypeHandler>> {
        self.handlers.insert(priority, Arc::new(handler))
    }

    pub(crate) fn remove_handler(&mut self, priority: usize) -> Option<Arc<DatatypeHandler>> {
        self.handlers.remove(&priority)
    }

    fn snapshot(&self, event: HandlerEvent) -> Option<HandlerNotification> {
        if self.handlers.is_empty() {
            return None;
        }
        Some(HandlerNotification {
            target: self.attr.datatype_target()?,
            handle: self.attr.client_common.handle.clone(),
            handlers: self.handlers.iter().map(|(&k, v)| (k, v.clone())).collect(),
            span: Span::current(),
            event,
        })
    }

    pub(crate) fn prepare_state_change(
        &self,
        old_state: DatatypeState,
        new_state: DatatypeState,
    ) -> Option<HandlerNotification> {
        self.snapshot(HandlerEvent::StateChange(old_state, new_state))
    }

    pub(crate) fn prepare_error(&self, err: DatatypeError) -> Option<HandlerNotification> {
        self.snapshot(HandlerEvent::Error(err))
    }
}

enum HandlerEvent {
    StateChange(DatatypeState, DatatypeState),
    Error(DatatypeError),
}

/// An immutable notification prepared under mutable state, dispatched after unlocking.
pub(crate) struct HandlerNotification {
    target: DatatypeTarget,
    handle: Handle,
    handlers: Vec<(usize, Arc<DatatypeHandler>)>,
    span: Span,
    event: HandlerEvent,
}

impl HandlerNotification {
    pub(crate) fn dispatch(self) {
        let Some(ds) = self.target.resolve() else {
            return;
        };
        let event_name = match self.event {
            HandlerEvent::StateChange(..) => "notify_state_change",
            HandlerEvent::Error(_) => "notify_error",
        };
        self.handle.spawn(async move {
            for (priority, handler) in self.handlers {
                self.span.in_scope(|| {
                    add_span_event!(format!("begin {event_name} priority={priority}"));
                    match &self.event {
                        HandlerEvent::StateChange(old, new) => {
                            handler.notify_state_change(ds.clone(), *old, *new);
                        }
                        HandlerEvent::Error(err) => handler.notify_error(ds.clone(), err.clone()),
                    }
                    add_span_event!(format!("end {event_name} priority={priority}"));
                });
            }
        });
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

    use tracing::{info, instrument};

    use crate::{
        Client, Datatype, DatatypeError, DatatypeHandler, DatatypeSet, DatatypeState,
        LocalConnectivity,
        utils::test_utils::{get_test_collection_name, get_test_func_name},
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
                assert_eq!(old_state, DatatypeState::Creating);
                assert_eq!(new_state, DatatypeState::Subscribed);
            });

        let handler2 =
            DatatypeHandler::new().set_on_state_change(move |_ds, old_state, new_state| {
                let a = count_for_h2.fetch_add(1, Ordering::Relaxed);
                assert_eq!(a, 1);
                assert_eq!(old_state, DatatypeState::Creating);
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

    #[test]
    #[instrument]
    fn can_report_removal_while_a_notification_is_in_flight() {
        // `dispatch` hands every notification its own `Arc` clone of the handler, so a
        // handler unset while one is still running is not solely owned by the manager.
        // Removal must be reported on the map entry, not on sole ownership.
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build()
            .unwrap();
        let counter = client
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();

        // `entered` reports that the dispatched notification is running and holding a
        // clone; `release` keeps it there until the unset below has been observed.
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let release = Arc::new(std::sync::Barrier::new(2));
        let release_in_handler = release.clone();
        counter.set_handler(
            1,
            DatatypeHandler::new().set_on_state_change(move |_ds, _old, _new| {
                let _ = entered_tx.send(());
                release_in_handler.wait();
            }),
        );

        counter.sync().unwrap();
        entered_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("the handler must be dispatched");

        assert!(
            counter.unset_handler(1),
            "a handler removed from the map is reported as removed even while notifying"
        );
        release.wait();

        assert!(
            !counter.unset_handler(1),
            "a second unset at the same priority removes nothing"
        );
    }

    #[test]
    #[instrument]
    fn can_pass_a_variable_datatype_set_to_the_handler() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build()
            .unwrap();

        let call_count = Arc::new(AtomicUsize::new(0));
        let count_for_handler = call_count.clone();

        let handler = DatatypeHandler::new().set_on_state_change(move |ds, _, _| {
            assert!(matches!(ds, DatatypeSet::Variable(_)));
            count_for_handler.fetch_add(1, Ordering::Relaxed);
        });

        let variable = client
            .create_datatype(get_test_func_name!())
            .with_handler(0, handler)
            .build_variable()
            .unwrap();

        variable.sync().unwrap();
        awaitility::at_most(Duration::from_secs(2))
            .poll_interval(Duration::from_micros(100))
            .until(|| call_count.load(Ordering::Relaxed) == 1);
    }

    #[test]
    #[instrument]
    fn can_notify_error() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let client = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build()
            .unwrap();

        let call_count = Arc::new(AtomicUsize::new(0));
        let count_for_h1 = call_count.clone();
        let count_for_h2 = call_count.clone();

        let handler1 = DatatypeHandler::new().set_on_error(move |ds, err| {
            info!("error1: {:?}", err);
            assert!(matches!(err, DatatypeError::ServerRejected(_)));
            let a = count_for_h1.fetch_add(1, Ordering::Relaxed);
            assert_eq!(a, 1);
            assert_eq!(ds.get_state(), DatatypeState::Disabled);
        });

        let handler2 = DatatypeHandler::new().set_on_error(move |ds, err| {
            info!("error2: {:?}", err);
            assert!(matches!(err, DatatypeError::ServerRejected(_)));
            let a = count_for_h2.fetch_add(1, Ordering::Relaxed);
            assert_eq!(a, 0);
            assert_eq!(ds.get_state(), DatatypeState::Disabled);
        });

        let counter1 = client
            .subscribe_datatype(get_test_func_name!())
            .with_handler(1, handler1)
            .with_handler(0, handler2)
            .build_counter()
            .unwrap();
        assert_eq!(counter1.get_state(), DatatypeState::Subscribing);
        assert!(matches!(
            counter1.sync().unwrap_err(),
            DatatypeError::ServerRejected(_)
        ));
        assert_eq!(counter1.get_state(), DatatypeState::Disabled);
        awaitility::at_most(Duration::from_secs(2))
            .poll_interval(Duration::from_micros(100))
            .until(|| call_count.load(Ordering::Relaxed) == 2);
    }
}
