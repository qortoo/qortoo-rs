use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use derive_more::Display;
use tokio::sync::oneshot;
use tracing::{Instrument, error, instrument};

use crate::{
    DatatypeError, connectivity::Connectivity, datatypes::wired::WiredDatatype,
    errors::with_err_out, observability::macros::add_span_event,
};

#[derive(Display)]
pub enum Event {
    #[display("Stop")]
    Stop(oneshot::Sender<()>),
    #[display("PushTransaction")]
    PushTransaction,
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
    pub fn new_arc(connectivity: Arc<dyn Connectivity>) -> Arc<Self> {
        let (unbounded_tx, unbounded_rx) = crossbeam_channel::unbounded::<Event>();
        let (bounded_tx, bounded_rx) = crossbeam_channel::bounded::<Event>(0);
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
            qortoo.col=%wired.attr.client_common.collection,
            qortoo.cl=%wired.attr.client_common.alias,
            qortoo.cuid=%wired.attr.client_common.cuid,
            qortoo.dt=%wired.attr.key,
            qortoo.duid=%wired.attr.get_duid(),
        )
    )]
    pub fn run(&self, wired: Arc<WiredDatatype>) {
        let unbounded_rx = self.unbounded_rx.clone();
        let bounded_rx = self.bounded_rx.clone();
        let rt_handle = wired.attr.client_common.handle.clone();
        let unbounded_tx = self.unbounded_tx.clone();
        rt_handle.spawn(
            async move {
                let connectivity = wired.attr.client_common.connectivity.clone();
                connectivity.register(wired.clone(), unbounded_tx);
                add_span_event!("start event_loop");
                loop {
                    wired.push_if_needed();
                    let event = crossbeam_channel::select! {
                        recv(unbounded_rx) -> msg => {
                            match msg {
                                Ok(event) => event,
                                Err(e) => {
                                    error!("unbounded channel error {e:?}; stopping event loop");
                                    break;
                                }
                            }
                        }
                        recv(bounded_rx) -> msg => {
                            match msg {
                                Ok(event) => event,
                                Err(e) => {
                                    error!("bounded channel error {e:?}; stopping event loop");
                                    break;
                                }
                            }
                        }
                    };
                    match event {
                        Event::Stop(tx) => {
                            add_span_event!("receive STOP");
                            if tx.send(()).is_err() {
                                error!("failed to send stop confirmation");
                            }
                            break;
                        }
                        Event::PushTransaction => {
                            add_span_event!("receive PushTransaction");
                            if let Err(e) = wired.push_pull() {
                                error!("push_pull failed: {e}");
                            }
                        }
                    }
                }
                add_span_event!("quiting event_loop");
            }
            .in_current_span(),
        );
    }

    pub fn send_stop(&self) {
        let (tx, rx) = oneshot::channel();
        match self.send_to_unbounded(Event::Stop(tx)) {
            Ok(_) => {
                futures::executor::block_on(async {
                    if rx.await.is_err() {
                        error!("failed to receive stop confirmation")
                    }
                });
            }
            Err(e) => {
                error!("failed to send stop event to event loop: {}", e);
            }
        }
    }

    fn send_to_unbounded(&self, ev: Event) -> Result<(), DatatypeError> {
        self.unbounded_tx
            .try_send(ev)
            .map_err(|e| with_err_out!(DatatypeError::FailureInEventLoop(Box::new(e))))
    }

    fn send_to_bounded(&self, ev: Event) -> Result<(), DatatypeError> {
        let ev_str = format!("{ev}");
        self.bounded_tx.try_send(ev).map_err(|e| {
            add_span_event!(ev_str, "result"=>"fail");
            DatatypeError::FailureInEventLoop(Box::new(e))
        })?;
        add_span_event!(ev_str, "result"=>"succeed");
        Ok(())
    }

    pub fn send_push_transaction_with_best_effort(&self) {
        if !self.connectivity.is_realtime() {
            return;
        }
        self.send_to_bounded(Event::PushTransaction)
            .unwrap_or_default();
    }

    pub fn send_push_transaction_with_guarantee(&self) {
        self.send_to_unbounded(Event::PushTransaction)
            .unwrap_or_default();
    }
}
