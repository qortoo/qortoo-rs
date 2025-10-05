use std::sync::Arc;

use crossbeam_channel::{Receiver, Sender};
use derive_more::Display;
use tracing::{Instrument, Span, debug, error, instrument};
use tracing_opentelemetry::OpenTelemetrySpanExt;

use crate::datatypes::transactional::TransactionalDatatype;

#[derive(Display)]
pub enum Event {
    #[display("Stop")]
    Stop,
    #[display("PushTransaction")]
    PushTransaction,
}

#[derive(Debug)]
pub struct EventLoop {
    event_tx: Sender<Event>,
    event_rx: Receiver<Event>,
}

impl EventLoop {
    pub fn new_arc() -> Arc<Self> {
        let (event_tx, event_rx) = crossbeam_channel::unbounded::<Event>();
        Arc::new(Self { event_tx, event_rx })
    }

    #[instrument(skip_all, name="datatype_event_loop",
        fields(
            syncyam.col=%arc_td.attr.client_common.collection,
            syncyam.cl=%arc_td.attr.client_common.alias,
            syncyam.cuid=%arc_td.attr.client_common.cuid,
            syncyam.dt=%arc_td.attr.key,
            syncyam.duid=%arc_td.attr.duid,
        )
    )]
    pub fn run(&self, arc_td: Arc<TransactionalDatatype>) {
        let event_rx = self.event_rx.clone();
        let arc_dt_for_thread = arc_td.clone();
        let event_loop_span = Span::current();
        arc_dt_for_thread.attr.client_common.handle.spawn(
            async move {
                event_loop_span.in_scope(|| {
                    debug!("started event loop");
                    loop {
                        let event = match event_rx.recv() {
                            Ok(event) => event,
                            Err(e) => {
                                error!("something wrong with {e:?}; stopping event loop");
                                break;
                            }
                        };
                        match event {
                            Event::Stop => {
                                break;
                            }
                            Event::PushTransaction => {
                                event_loop_span.add_event("push transaction", vec![]);
                            }
                        }
                    }
                    event_loop_span.add_event("stop event_loop", vec![]);
                });
            }
            .instrument(Span::current()),
        );
    }

    pub fn send_stop(&self) {
        self.send(Event::Stop)
    }

    fn send(&self, ev: Event) {
        if let Err(e) = self.event_tx.send(ev) {
            error!("sending error during event loop: {e}");
            // TODO: When this happen?, what should be done?
        }
    }

    pub fn send_push_transaction(&self) {
        self.send(Event::PushTransaction)
    }
}
