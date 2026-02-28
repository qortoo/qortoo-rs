mod tests_datatype_handler {
    use std::time::Duration;

    use DatatypeSet::Counter;
    use qortoo::{Client, Datatype, DatatypeHandler, DatatypeSet, DatatypeState};
    use tracing::{info, instrument};

    #[test]
    #[instrument]
    fn can_use_datatype_handler() {
        let client = Client::builder("test-collection", "can_use_datatype_handler")
            .build()
            .unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        let tx_priority_0 = tx.clone();

        let handler0 = DatatypeHandler::new().set_on_state_change(move |ds, old, new| {
            let Counter(counter) = ds;
            assert_ne!(old, new);
            assert_eq!(counter.get_state(), new);
            assert_eq!(counter.get_key(), "counter-1");
            info!("{} {}", old, new);
            tx_priority_0.send((0, counter.get_value())).unwrap();
        });

        let handler1 = DatatypeHandler::new().set_on_state_change(move |ds, _, _| {
            let Counter(counter) = ds;
            tx.send((1, counter.get_value())).unwrap();
        });

        let counter = client
            .create_datatype("counter-1")
            .with_handler(0, handler0)
            .with_handler(1, handler1)
            .build_counter()
            .unwrap();

        counter.increase_by(42).unwrap();
        let first = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        let second = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!([first.0, second.0], [0, 1]);
        assert_eq!(counter.get_value(), first.1);
        assert_eq!(counter.get_value(), second.1);
        assert!(rx.recv_timeout(Duration::from_millis(100)).is_err());
        assert_eq!(counter.get_state(), DatatypeState::Subscribed);
    }
}
