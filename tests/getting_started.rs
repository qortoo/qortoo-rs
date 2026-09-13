//! Mirrors the three walkthrough snippets in `docs/getting-started.md`, verbatim. These
//! tests exist to keep that document accurate, not to prove new CRDT behavior — the
//! underlying semantics already have their own unit tests. If one of these starts
//! failing, the fix is almost always to update `docs/getting-started.md` to match,
//! not to change the test.

mod tests_getting_started {
    use qortoo::{Client, Datatype, LocalConnectivity};
    use serde_json::Value;
    use tracing::instrument;

    #[test]
    #[instrument]
    fn can_run_the_getting_started_counter_walkthrough() {
        let client = Client::builder("my-collection", "my-client")
            .build()
            .unwrap();

        let counter = client
            .create_datatype("my-counter")
            .build_counter()
            .unwrap();

        counter.increase().unwrap();
        counter.increase_by(5).unwrap();
        assert_eq!(counter.get_value(), 6);
    }

    #[test]
    #[instrument]
    fn can_run_the_getting_started_variable_walkthrough() {
        let client = Client::builder("my-collection", "my-other-client")
            .build()
            .unwrap();

        let name = client.create_datatype("my-name").build_variable().unwrap();
        assert_eq!(name.get::<Value>().unwrap(), Value::Null);

        name.set(&"ada").unwrap();
        assert_eq!(name.get::<String>().unwrap(), "ada");
    }

    #[test]
    #[instrument]
    fn can_run_the_getting_started_two_client_sync_walkthrough() {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false); // require explicit sync() calls

        let client1 = Client::builder("my-collection", "client-1")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client2 = Client::builder("my-collection", "client-2")
            .with_connectivity(connectivity)
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype("shared-counter")
            .build_counter()
            .unwrap();
        counter1.increase().unwrap();
        counter1.sync().unwrap(); // push what client1 has

        let counter2 = client2
            .subscribe_datatype("shared-counter")
            .build_counter()
            .unwrap();
        counter2.sync().unwrap(); // pull what client1 pushed

        assert_eq!(counter2.get_value(), 1);
    }
}
