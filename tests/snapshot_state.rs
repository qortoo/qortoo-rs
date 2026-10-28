use qortoo::{Client, Datatype, LocalConnectivity};
use rstest::rstest;

#[rstest]
#[case::before_creator_sync(false)]
#[case::after_creator_sync(true)]
fn can_exclude_unsynced_counter_changes_from_subscription(#[case] sync_creator: bool) {
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let creator = Client::builder("snapshot-counter", "creator")
        .with_connectivity(connectivity.clone())
        .build()
        .unwrap();
    let subscriber = Client::builder("snapshot-counter", "subscriber")
        .with_connectivity(connectivity)
        .build()
        .unwrap();
    let source = creator.create_datatype("counter").build_counter().unwrap();
    source.increase_by(7).unwrap();
    source.sync().unwrap();
    source.increase_by(5).unwrap();
    assert_eq!(source.get_value(), 12);

    let replica = subscriber
        .subscribe_datatype("counter")
        .build_counter()
        .unwrap();
    replica.sync().unwrap();
    assert_eq!(source.get_value(), 12);

    if sync_creator {
        source.sync().unwrap();
        replica.sync().unwrap();
        assert_eq!(
            replica.get_value(),
            12,
            "the pending increase must apply once"
        );
    } else {
        assert_eq!(replica.get_value(), 7, "only synced changes may be exposed");
    }
}

#[test]
fn can_exclude_unsynced_variable_value_from_subscription() {
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let creator = Client::builder("snapshot-variable", "creator")
        .with_connectivity(connectivity.clone())
        .build()
        .unwrap();
    let subscriber = Client::builder("snapshot-variable", "subscriber")
        .with_connectivity(connectivity)
        .build()
        .unwrap();
    let source = creator
        .create_datatype("variable")
        .build_variable()
        .unwrap();
    source.set(&"synced").unwrap();
    source.sync().unwrap();
    source.set(&"pending").unwrap();

    let replica = subscriber
        .subscribe_datatype("variable")
        .build_variable()
        .unwrap();
    replica.sync().unwrap();
    assert_eq!(source.get::<String>().unwrap(), "pending");
    assert_eq!(replica.get::<String>().unwrap(), "synced");
}

#[rstest]
#[case::different_pending_value("pending")]
#[case::same_value_higher_pending_timestamp("initial")]
fn can_preserve_remote_variable_winner_hidden_by_unsynced_changes(#[case] pending: &str) {
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let creator = Client::builder("snapshot-variable-winner", "creator")
        .with_connectivity(connectivity.clone())
        .build()
        .unwrap();
    let writer = Client::builder("snapshot-variable-winner", "writer")
        .with_connectivity(connectivity.clone())
        .build()
        .unwrap();
    let subscriber = Client::builder("snapshot-variable-winner", "subscriber")
        .with_connectivity(connectivity)
        .build()
        .unwrap();
    let source = creator
        .create_datatype("variable")
        .build_variable()
        .unwrap();
    source.set(&"initial").unwrap();
    source.sync().unwrap();
    let remote = writer
        .subscribe_datatype("variable")
        .build_variable()
        .unwrap();
    remote.sync().unwrap();

    // A strictly higher pending timestamp must not suppress synced history on a new replica.
    source.set(&pending).unwrap();
    source.set(&pending).unwrap();
    source.set(&pending).unwrap();
    remote.set(&"remote").unwrap();
    remote.sync().unwrap();
    assert_eq!(remote.get::<String>().unwrap(), "remote");

    let replica = subscriber
        .subscribe_datatype("variable")
        .build_variable()
        .unwrap();
    replica.sync().unwrap();
    assert_eq!(source.get::<String>().unwrap(), pending);
    assert_eq!(replica.get::<String>().unwrap(), "remote");
}

#[test]
fn can_continue_variable_writes_after_a_backend_snapshot() {
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let creator = Client::builder("snapshot-variable-future", "creator")
        .with_connectivity(connectivity.clone())
        .build()
        .unwrap();
    let writer = Client::builder("snapshot-variable-future", "writer")
        .with_connectivity(connectivity.clone())
        .build()
        .unwrap();
    let observer = Client::builder("snapshot-variable-future", "observer")
        .with_connectivity(connectivity)
        .build()
        .unwrap();
    let source = creator
        .create_datatype("variable")
        .build_variable()
        .unwrap();
    source.set(&7).unwrap();
    source.sync().unwrap();
    source.set(&20).unwrap();
    let remote = writer
        .subscribe_datatype("variable")
        .build_variable()
        .unwrap();
    remote.sync().unwrap();
    remote.set(&9).unwrap();
    remote.set(&11).unwrap();
    remote.sync().unwrap();
    source.sync().unwrap();
    remote.sync().unwrap();
    assert_eq!(source.get::<i64>().unwrap(), 11);
    assert_eq!(remote.get::<i64>().unwrap(), 11);

    let replica = observer
        .subscribe_datatype("variable")
        .build_variable()
        .unwrap();
    replica.sync().unwrap();
    assert_eq!(replica.get::<i64>().unwrap(), 11);
    assert_eq!(replica.get_client_version(), 0);
    replica.set(&13).unwrap();
    replica.sync().unwrap();
    assert_eq!(replica.get_synced_client_version(), 1);
    source.sync().unwrap();
    remote.sync().unwrap();
    assert_eq!(source.get::<i64>().unwrap(), 13);
    assert_eq!(remote.get::<i64>().unwrap(), 13);
    assert_eq!(replica.get::<i64>().unwrap(), 13);
}
