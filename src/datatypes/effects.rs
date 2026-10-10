use crate::datatypes::{handler::HandlerNotification, target::DatatypeTarget};

pub(crate) enum DatatypeEffect {
    Notify(HandlerNotification),
    Detach(DatatypeTarget),
}

/// Effects reserved by one mutable-state operation, executed after unlocking.
#[derive(Default)]
pub(crate) struct DatatypeEffectBatch(Vec<DatatypeEffect>);

impl std::fmt::Debug for DatatypeEffectBatch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DatatypeEffectBatch")
            .field("pending", &self.0.len())
            .finish()
    }
}

impl DatatypeEffectBatch {
    pub(crate) fn push(&mut self, effect: DatatypeEffect) {
        self.0.push(effect);
    }

    /// The caller holds the shared effects gate across mutation and dispatch,
    /// but releases mutable state before dispatching. Callbacks run asynchronously.
    pub(crate) fn dispatch(self) {
        for effect in self.0 {
            match effect {
                DatatypeEffect::Notify(notification) => notification.dispatch(),
                DatatypeEffect::Detach(target) => target.detach(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, mpsc},
        time::Duration,
    };

    use crate::{
        Client, Datatype, DatatypeHandler, DatatypeState, LocalConnectivity,
        datatypes::datatype::DatatypeBlanket,
        utils::test_utils::{get_test_collection_name, get_test_func_name},
    };

    const TIMEOUT: Duration = Duration::from_secs(5);

    fn manual_client() -> Client {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity)
            .build()
            .unwrap()
    }

    #[test]
    fn can_keep_handler_snapshots_across_replacement_and_removal_before_dispatch() {
        let client = manual_client();
        let counter = client.create_datatype("counter").build_counter().unwrap();
        let (old_tx, old_rx) = mpsc::channel();
        let (new_tx, new_rx) = mpsc::channel();
        counter.set_handler(
            0,
            DatatypeHandler::new().set_on_state_change(move |_, old, new| {
                old_tx.send((old, new)).unwrap();
            }),
        );
        let first = {
            let mut mutable = counter.get_core().core.mutable.write();
            mutable.set_state(DatatypeState::Subscribed);
            assert!(old_rx.try_recv().is_err());
            mutable.take_effects()
        };
        counter.set_handler(
            0,
            DatatypeHandler::new().set_on_state_change(move |_, old, new| {
                new_tx.send((old, new)).unwrap();
            }),
        );
        let second = {
            let mut mutable = counter.get_core().core.mutable.write();
            mutable.set_state(DatatypeState::Unsubscribing);
            mutable.set_state(DatatypeState::Unsubscribing); // No duplicate notification.
            mutable.take_effects()
        };
        assert!(counter.unset_handler(0));
        first.dispatch();
        second.dispatch();
        assert_eq!(
            old_rx.recv_timeout(TIMEOUT).unwrap(),
            (DatatypeState::Creating, DatatypeState::Subscribed)
        );
        assert_eq!(
            new_rx.recv_timeout(TIMEOUT).unwrap(),
            (DatatypeState::Subscribed, DatatypeState::Unsubscribing)
        );
        assert!(old_rx.recv_timeout(TIMEOUT).is_err());
        assert!(new_rx.recv_timeout(TIMEOUT).is_err());
    }

    #[test]
    fn can_keep_replacement_registered_when_old_instance_effects_run() {
        let client = manual_client();
        let original = client.create_datatype("counter").build_counter().unwrap();
        let original_id = original.get_core() as *const _ as usize;
        let (notify_tx, notify_rx) = mpsc::channel();
        original.set_handler(
            0,
            DatatypeHandler::new().set_on_state_change(move |ds, _, new| {
                notify_tx.send((ds.get_core_id(), new)).unwrap();
            }),
        );
        let effects = {
            let mut mutable = original.get_core().core.mutable.write();
            mutable.set_state(DatatypeState::Disabled);
            mutable.take_effects()
        };
        original
            .get_core()
            .core
            .attr
            .client_common
            .detach_datatype_if_same_instance("counter", original_id);
        let replacement = client.create_datatype("counter").build_counter().unwrap();
        let replacement_id = replacement.get_core() as *const _ as usize;
        assert_ne!(original_id, replacement_id);
        effects.dispatch();
        assert_eq!(
            notify_rx.recv_timeout(TIMEOUT).unwrap(),
            (original_id, DatatypeState::Disabled)
        );
        assert_eq!(
            client.get_datatype("counter").unwrap().get_core_id(),
            replacement_id
        );
        assert_eq!(replacement.get_state(), DatatypeState::Creating);
    }

    #[rstest::rstest]
    #[case(false)]
    #[case(true)]
    fn can_drop_replaced_or_removed_handlers_after_unlocking(#[case] replace: bool) {
        struct DropProbe {
            core: std::sync::Weak<crate::datatypes::core::DatatypeCore>,
            result: mpsc::Sender<bool>,
        }
        impl Drop for DropProbe {
            fn drop(&mut self) {
                let available = self.core.upgrade().unwrap().mutable.try_write().is_some();
                self.result.send(available).unwrap();
            }
        }
        let client = manual_client();
        let counter = client.create_datatype("counter").build_counter().unwrap();
        let (result_tx, result_rx) = mpsc::channel();
        let probe = DropProbe {
            core: Arc::downgrade(&counter.get_core().core),
            result: result_tx,
        };
        counter.set_handler(
            0,
            DatatypeHandler::new().set_on_state_change(move |_, _, _| {
                let _ = &probe;
            }),
        );
        if replace {
            counter.set_handler(0, DatatypeHandler::new());
        } else {
            assert!(counter.unset_handler(0));
        }
        assert!(result_rx.recv_timeout(TIMEOUT).unwrap());
    }

    #[test]
    fn can_drop_an_instance_with_a_prepared_notification() {
        let client = manual_client();
        let counter = client.create_datatype("counter").build_counter().unwrap();
        let (notify_tx, notify_rx) = mpsc::channel();
        counter.set_handler(
            0,
            DatatypeHandler::new().set_on_state_change(move |_, _, _| {
                notify_tx.send(()).unwrap();
            }),
        );
        let target = counter
            .get_core()
            .core
            .attr
            .weak_transactional
            .read()
            .clone()
            .unwrap();
        let effects = {
            let mut mutable = counter.get_core().core.mutable.write();
            mutable.set_state(DatatypeState::Subscribed);
            mutable.take_effects()
        };
        drop(client);
        drop(counter);
        assert!(target.upgrade().is_none());
        effects.dispatch();
        assert!(notify_rx.try_recv().is_err());
    }
}
