//! Public callback progress and lifetime contracts, independent of OS resource budgets.
use std::time::Duration;

use crossbeam_channel::{Sender, bounded};
use qortoo::{Client, Datatype, DatatypeHandler, DatatypeSet, DatatypeState, LocalConnectivity};

const DEADLINE: Duration = Duration::from_secs(5);

#[test]
fn can_read_write_transact_sync_and_unsubscribe_from_a_handler() {
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let client = Client::builder("runtime-contract", "reentrant-handler")
        .with_connectivity(connectivity)
        .build()
        .unwrap();
    let (done_tx, done_rx) = bounded(1);
    let handler = DatatypeHandler::new().set_on_state_change(move |ds, _, new| {
        if new != DatatypeState::Subscribed {
            return;
        }
        let DatatypeSet::Counter(counter) = ds else {
            panic!("expected counter");
        };
        assert_eq!(counter.get_value(), 0);
        counter.increase().unwrap();
        counter
            .transaction("handler", |c| {
                c.increase_by(2)?;
                Ok(())
            })
            .unwrap();
        counter.sync().unwrap();
        assert_eq!(counter.get_value(), 3);
        counter.unsubscribe().unwrap();
        counter.sync().unwrap();
        assert_eq!(counter.get_state(), DatatypeState::Disabled);
        done_tx.send(()).unwrap();
    });
    let counter = client
        .create_datatype("counter")
        .with_handler(0, handler)
        .build_counter()
        .unwrap();
    counter.sync().unwrap();
    done_rx.recv_timeout(DEADLINE).unwrap();
    assert!(client.get_datatype("counter").is_none());
}

struct DropSignal(Sender<()>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        self.0.send(()).unwrap();
    }
}

#[test]
fn can_keep_a_blocked_callback_alive_after_external_handles_are_dropped() {
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let client = Client::builder("runtime-contract", "blocked-handler")
        .with_connectivity(connectivity)
        .build()
        .unwrap();
    let (entered_tx, entered_rx) = bounded(1);
    let (release_tx, release_rx) = bounded(1);
    let (later_tx, later_rx) = bounded(1);
    let (dropped_tx, dropped_rx) = bounded(1);
    let lifetime = DropSignal(dropped_tx);
    let handler = DatatypeHandler::new().set_on_state_change(move |ds, _, new| {
        let _keep_signal_alive = &lifetime;
        if new != DatatypeState::Subscribed {
            return;
        }
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(DEADLINE).unwrap();
        let DatatypeSet::Counter(counter) = ds else {
            panic!("expected counter");
        };
        // A later transition may have occurred while this notification was blocked.
        assert_eq!(counter.get_value(), 3);
        assert_eq!(counter.get_state(), DatatypeState::Disabled);
    });
    let later = DatatypeHandler::new().set_on_state_change(move |_, _, new| {
        if new == DatatypeState::Subscribed {
            later_tx.send(()).unwrap();
        }
    });
    let counter = client
        .create_datatype("counter")
        .with_handler(0, handler)
        .with_handler(1, later)
        .build_counter()
        .unwrap();
    counter.sync().unwrap();
    entered_rx.recv_timeout(DEADLINE).unwrap();
    // A blocked callback holds neither mutable state nor the transaction/effects gates.
    counter.increase().unwrap();
    counter
        .transaction("outside", |c| {
            c.increase_by(2)?;
            Ok(())
        })
        .unwrap();
    counter.sync().unwrap();
    counter.unsubscribe().unwrap();
    counter.sync().unwrap();
    assert!(later_rx.try_recv().is_err());
    // Stop further snapshots. The in-flight notification still owns both handlers.
    assert!(counter.unset_handler(0));
    assert!(counter.unset_handler(1));
    drop(client);
    drop(counter);
    assert!(dropped_rx.try_recv().is_err());
    release_tx.send(()).unwrap();
    later_rx.recv_timeout(DEADLINE).unwrap();
    dropped_rx.recv_timeout(DEADLINE).unwrap();
}
