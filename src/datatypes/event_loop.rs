use std::{sync::Arc, time::Duration};

use backon::{BackoffBuilder, ExponentialBackoff, ExponentialBuilder};
use crossbeam_channel::{Receiver, Sender};
use derive_more::Display;
use tokio::sync::oneshot;
use tracing::{Span, error, instrument};

use crate::{
    DatatypeError,
    connectivity::Connectivity,
    datatypes::wired::WiredDatatype,
    defaults::DEFAULT_EVENT_LOOP_TIMEOUT_MS,
    errors::{
        datatypes::{DatatypeAction, EventLoopAction},
        with_err_out,
    },
    observability::{metrics, trace::add_span_event},
    types::notification::Notification,
};

const BACKOFF_MIN_DELAY: Duration = Duration::from_millis(500);
const BACKOFF_MAX_DELAY: Duration = Duration::from_secs(30);

#[derive(Display)]
pub enum Event {
    #[display("Stop")]
    Stop(Sender<()>),
    #[display("PushTransaction")]
    PushTransaction(Option<oneshot::Sender<Option<DatatypeError>>>),
    #[display("BackOff")]
    BackOff,
    #[display("Notify")]
    Notify(Notification),
}

#[derive(Debug)]
pub struct EventLoop {
    connectivity: Arc<dyn Connectivity>,
    bounded_tx: Sender<Event>,
    bounded_rx: Receiver<Event>,
    unbounded_tx: Sender<Event>,
    unbounded_rx: Receiver<Event>,
}

impl EventLoop {
    fn build_backoff() -> ExponentialBackoff {
        ExponentialBuilder::new()
            .with_min_delay(BACKOFF_MIN_DELAY)
            .with_max_delay(BACKOFF_MAX_DELAY)
            .without_max_times()
            .build()
    }

    pub fn new_arc(connectivity: Arc<dyn Connectivity>) -> Arc<Self> {
        let (unbounded_tx, unbounded_rx) = crossbeam_channel::unbounded::<Event>();
        let (bounded_tx, bounded_rx) = crossbeam_channel::bounded::<Event>(1);
        Arc::new(Self {
            connectivity,
            unbounded_rx,
            unbounded_tx,
            bounded_tx,
            bounded_rx,
        })
    }

    #[instrument(skip_all, name="datatype_event_loop",
        fields(
            collection=%wired.attr.client_common.collection,
            client=%wired.attr.client_common.alias,
            cuid=%wired.attr.client_common.cuid,
            data_key=%wired.attr.key,
            duid=%wired.attr.get_duid(),
        )
    )]
    pub fn run(&self, wired: Arc<WiredDatatype>) {
        let unbounded_rx = self.unbounded_rx.clone();
        let bounded_rx = self.bounded_rx.clone();
        let rt_handle = wired.attr.client_common.handle.clone();
        let unbounded_tx = self.unbounded_tx.clone();
        let bounded_tx = self.bounded_tx.clone();
        let connectivity = wired.attr.client_common.connectivity.clone();
        let span = Span::current();
        connectivity.register(wired.clone(), unbounded_tx);
        rt_handle.spawn_blocking(move || {
            span.in_scope(|| {
                add_span_event!("start event_loop");
                let mut event_loop_action = EventLoopAction::Normal;
                let mut backoff = None;
                loop {
                    match Self::receive_event(
                        &wired,
                        &bounded_rx,
                        &unbounded_rx,
                        &mut event_loop_action,
                        &mut backoff,
                    ) {
                        Ok(event) => match event {
                            Event::Stop(tx) => {
                                add_span_event!("receive STOP");
                                if tx.send(()).is_err() {
                                    error!("failed to respond STOP event");
                                }
                                break;
                            }
                            Event::PushTransaction(resp_tx) => {
                                if matches!(event_loop_action, EventLoopAction::PauseSync) {
                                    Self::process_blocking_resp(
                                        resp_tx,
                                        Some(DatatypeError::FailedInEventLoop(
                                            "event loop paused".into(),
                                        )),
                                    );
                                    continue;
                                }
                                let opt_datatype_error = match wired.push_pull() {
                                    Ok(_) => {
                                        event_loop_action = EventLoopAction::Normal;
                                        None
                                    }
                                    Err(dewa) => {
                                        event_loop_action = dewa.event_loop_action;
                                        if matches!(event_loop_action, EventLoopAction::BackOff) {
                                            metrics::emit_backoff(&wired.attr);
                                        }
                                        wired
                                            .handle_error(dewa.error.clone(), dewa.datatype_action);
                                        Some(dewa.error)
                                    }
                                };
                                if !matches!(event_loop_action, EventLoopAction::BackOff) {
                                    backoff = None;
                                }
                                Self::process_blocking_resp(resp_tx, opt_datatype_error);
                            }
                            Event::BackOff => {}
                            Event::Notify(notify) => {
                                if wired.handle_notification(notify) {
                                    // best-effort: drop if a PushTransaction is already queued
                                    let _ = bounded_tx.try_send(Event::PushTransaction(None));
                                }
                            }
                        },
                        Err(err) => {
                            wired.handle_error(err, DatatypeAction::Disable);
                        }
                    }
                }
                add_span_event!("quiting event_loop");
            });
        });
    }

    fn process_blocking_resp(
        blocking_resp_tx: Option<oneshot::Sender<Option<DatatypeError>>>,
        opt_datatype_error: Option<DatatypeError>,
    ) {
        if let Some(sender) = blocking_resp_tx {
            if sender.send(opt_datatype_error).is_err() {
                error!("failed to respond PushTransaction event");
            }
        }
    }

    #[instrument(skip_all)]
    fn receive_event(
        wired: &WiredDatatype,
        bounded_rx: &Receiver<Event>,
        unbounded_rx: &Receiver<Event>,
        event_loop_action: &mut EventLoopAction,
        backoff: &mut Option<ExponentialBackoff>,
    ) -> Result<Event, DatatypeError> {
        let (push_if_needed, backoff_duration) = match event_loop_action {
            EventLoopAction::Normal => (true, None),
            EventLoopAction::PauseSync => (false, None),
            EventLoopAction::BackOff => {
                let backoff_iter = backoff.get_or_insert_with(Self::build_backoff);
                let d = backoff_iter.next().unwrap_or(BACKOFF_MAX_DELAY);
                (false, Some(d))
            }
        };

        if push_if_needed && wired.push_if_needed() {
            return Ok(Event::PushTransaction(None));
        }

        let map_err = |e, ch: &str| {
            let err_msg = format!("{ch} channel error {e:?}; stopping event loop");
            with_err_out!(DatatypeError::FailedInEventLoop(err_msg))
        };

        if let Some(duration) = backoff_duration {
            add_span_event!(format!("enter backOff during {duration:?}"));
            crossbeam_channel::select! {
                // only unbounded_rx is allowed to receive events during backoff for STOP or explicit sync event
                recv(unbounded_rx) -> event => event.map_err(|e| map_err(e, "unbounded")),
                default(duration) => {
                    *event_loop_action = EventLoopAction::Normal;
                    Ok(Event::BackOff)
                }
            }
        } else {
            crossbeam_channel::select! {
                recv(unbounded_rx) -> event => event.map_err(|e| map_err(e, "unbounded")),
                recv(bounded_rx) -> event => event.map_err(|e| map_err(e, "bounded")),
            }
        }
    }

    pub fn send_stop(&self) {
        let (tx, rx) = crossbeam_channel::bounded::<()>(1);
        match self.send_to_unbounded(Event::Stop(tx)) {
            Ok(_) => {
                if let Err(e) =
                    rx.recv_timeout(Duration::from_millis(DEFAULT_EVENT_LOOP_TIMEOUT_MS))
                {
                    error!("fail to stop event loop: {e}");
                }
            }
            Err(e) => {
                error!("failed to send stop event to event loop: {}", e);
            }
        }
    }

    fn send_to_unbounded(&self, ev: Event) -> Result<(), DatatypeError> {
        self.unbounded_tx
            .try_send(ev)
            .map_err(|e| with_err_out!(DatatypeError::FailedInEventLoop(format!("{e:?}"))))
    }

    fn send_to_bounded(&self, ev: Event) -> Result<(), DatatypeError> {
        let ev_str = format!("{ev}");
        self.bounded_tx.try_send(ev).map_err(|e| {
            add_span_event!(ev_str, "result"=>"fail");
            DatatypeError::FailedInEventLoop(format!("{e:?}"))
        })?;
        add_span_event!(ev_str, "result"=>"succeed");
        Ok(())
    }

    pub fn send_push_transaction_with_best_effort(&self) {
        if !self.connectivity.is_realtime() {
            return;
        }
        self.send_to_bounded(Event::PushTransaction(None))
            .unwrap_or_default();
    }

    pub fn send_push_transaction_with_guarantee(&self) -> Result<(), DatatypeError> {
        let (tx, rx) = oneshot::channel();
        self.send_to_unbounded(Event::PushTransaction(Some(tx)))?;
        futures::executor::block_on(async {
            match rx.await {
                Ok(Some(err)) => Err(err),
                Ok(None) => Ok(()),
                Err(e) => Err(DatatypeError::FailedInEventLoop(format!(
                    "failed to receive response of sync(): {e}"
                ))),
            }
        })
    }
}

#[cfg(test)]
mod tests_event_loop {
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use tracing::instrument;

    use crate::{
        Client, ConnectivityError, DatatypeError, DatatypeState,
        connectivity::local_connectivity::LocalConnectivity,
        datatypes::datatype::Datatype,
        errors::{datatypes::DatatypeErrorWithActions, push_pull::ClientPushPullError},
        utils::test_utils::{get_test_collection_name, get_test_func_name, get_test_ids},
    };

    fn make_backoff_error() -> DatatypeErrorWithActions {
        ClientPushPullError::FailedInConnectivity(ConnectivityError::ResourceNotFound(
            "injected".to_string(),
        ))
        .mapping()
    }

    fn make_pause_sync_error() -> DatatypeErrorWithActions {
        ClientPushPullError::FailedWithProtocolViolation("injected".to_string()).mapping()
    }

    /// Test that an explicit sync() call bypasses the BackOff wait via the unbounded channel.
    /// After a FailedInConnectivity error sets BackOff on the event loop, the next sync()
    /// is still processed immediately (not after the 500ms delay).
    #[test]
    #[instrument]
    fn can_manually_retry_after_backoff_error() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();
        let client = Client::builder(collection, "client")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter = client.create_datatype(key).build_counter().unwrap();

        let interceptor = connectivity
            .get_wired_interceptor(&resource_id, &client.get_cuid())
            .unwrap();

        // inject FailedInConnectivity → maps to EventLoopAction::BackOff, DatatypeAction::Normal
        interceptor.set_after_pull(|_| Err(make_backoff_error()));

        let err = counter.sync().unwrap_err();
        assert!(matches!(err, DatatypeError::FailedByClientPushPullError(_)));
        // DatatypeAction::Normal → no state change
        assert_eq!(counter.get_state(), DatatypeState::Creating);

        // explicit sync() sends to unbounded channel → bypasses 500ms BackOff wait
        interceptor.set_after_pull(|_| Ok(()));
        assert!(counter.sync().is_ok());
        assert_eq!(counter.get_state(), DatatypeState::Subscribed);
    }

    /// Test that the event loop auto-retries after the BackOff timeout expires (~500ms).
    /// Without any manual sync() call, the event loop's select! default(500ms) fires
    /// and triggers a retry push_pull().
    #[test]
    #[instrument]
    fn can_auto_retry_after_backoff_timeout() {
        let connectivity = LocalConnectivity::new_arc();
        let (collection, key, resource_id) = get_test_ids!();

        let client = Client::builder(collection, "client")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter = client.create_datatype(key).build_counter().unwrap();

        let interceptor = connectivity
            .get_wired_interceptor(&resource_id, &client.get_cuid())
            .unwrap();

        let pull_count = Arc::new(AtomicUsize::new(0));
        let pull_count_for_after_pull = pull_count.clone();
        interceptor.set_after_pull(move |_| {
            let count = pull_count_for_after_pull.fetch_add(1, Ordering::SeqCst) + 1;
            if count >= 3 {
                Ok(())
            } else {
                Err(make_backoff_error())
            }
        });

        assert_eq!(counter.get_state(), DatatypeState::Creating);

        awaitility::at_most(Duration::from_secs(10))
            .poll_interval(Duration::from_millis(100))
            .until(|| counter.get_state() == DatatypeState::Subscribed);
    }

    /// Test that a successful manual retry exits BackOff immediately.
    /// If BackOff is not cleared on success, the loop can fire one more timed retry.
    #[test]
    #[instrument]
    fn can_block_extra_retry_after_successful_manual_retry() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        let client = Client::builder(collection, "client")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter = client.create_datatype(key).build_counter().unwrap();

        let interceptor = connectivity
            .get_wired_interceptor(&resource_id, &client.get_cuid())
            .unwrap();

        let should_fail = Arc::new(AtomicBool::new(true));
        let should_fail_for_after_pull = should_fail.clone();
        let pull_count = Arc::new(AtomicUsize::new(0));
        let pull_count_for_after_pull = pull_count.clone();
        interceptor.set_after_pull(move |_| {
            pull_count_for_after_pull.fetch_add(1, Ordering::SeqCst);
            if should_fail_for_after_pull.load(Ordering::SeqCst) {
                Err(make_backoff_error())
            } else {
                Ok(())
            }
        });

        // First, sync fails
        assert!(counter.sync().is_err());
        assert_eq!(pull_count.load(Ordering::SeqCst), 1);

        // Then, make sync not fail
        should_fail.store(false, Ordering::SeqCst);

        assert!(counter.sync().is_ok());
        assert_eq!(pull_count.load(Ordering::SeqCst), 2);

        // Check if no sync happens
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(pull_count.load(Ordering::SeqCst), 2);
    }

    /// Test that PauseSync + Disable transitions datatype to Disabled and
    /// does not keep retrying automatically afterward.
    #[test]
    #[instrument]
    fn can_handle_pause_sync_error_without_auto_retry_loop() {
        let connectivity = LocalConnectivity::new_arc();
        let (collection, key, resource_id) = get_test_ids!();

        let client = Client::builder(collection, "client")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter = client.create_datatype(key).build_counter().unwrap();

        let interceptor = connectivity
            .get_wired_interceptor(&resource_id, &client.get_cuid())
            .unwrap();

        let pull_count = Arc::new(AtomicUsize::new(0));
        let pull_count_for_after_pull = pull_count.clone();
        interceptor.set_after_pull(move |_| {
            pull_count_for_after_pull.fetch_add(1, Ordering::SeqCst);
            Err(make_pause_sync_error())
        });

        assert!(counter.sync().is_err());
        assert_eq!(pull_count.load(Ordering::SeqCst), 1);
        assert_eq!(counter.get_state(), DatatypeState::Disabled);

        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(counter.get_state(), DatatypeState::Disabled);
        assert_eq!(pull_count.load(Ordering::SeqCst), 1);
    }
}
