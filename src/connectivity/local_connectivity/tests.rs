use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

use tracing::instrument;

use crate::{
    Client, ClientError, CollectionInfo, Datatype, DatatypeState,
    connectivity::Connectivity,
    utils::test_utils::{
        get_test_collection_name, get_test_func_name, get_test_ids, local_connectivity,
    },
};

#[test]
fn can_open_clients_in_multiple_collections_on_one_backend() {
    let backend = local_connectivity();
    backend.set_realtime(false);
    let limits = CollectionInfo {
        max_state_bytes: 10_000,
        max_key_bytes: 64,
        max_value_bytes: 512,
        forbidden_keys: vec!["_id".into(), "_rev".into()],
        max_snapshot_bytes: 20_000,
        max_transmission_bytes: 1_000,
    };
    backend
        .create_collection("configured", limits.clone())
        .unwrap();
    let client_builder =
        |collection, alias| Client::builder(collection, alias).with_connectivity(backend.clone());
    let first = client_builder("configured", "first").build().unwrap();
    let second = client_builder("configured", "second").build().unwrap();
    let default = client_builder("unconfigured", "third").build().unwrap();
    {
        let collections = backend.collections.read();
        assert!(collections["configured"].clients.is_empty());
        assert!(!collections.contains_key("unconfigured"));
    }
    assert_eq!(first.get_collection_info(), None);
    let builder = first.create_datatype("counter");
    assert_eq!(
        builder.build_counter().err(),
        Some(ClientError::ClientNotOpen)
    );
    assert!(first.get_datatype("counter").is_none());

    first.open().unwrap();
    second.open().unwrap();
    default.open().unwrap();
    first.create_datatype("counter").build_counter().unwrap();
    assert_eq!(first.get_collection_info(), Some(&limits));
    assert_eq!(second.get_collection_info(), Some(&limits));
    {
        let collections = backend.collections.read();
        assert_eq!(
            collections["configured"].clients,
            [first.get_cuid(), second.get_cuid()].into()
        );
        assert_eq!(
            collections["unconfigured"].clients,
            [default.get_cuid()].into()
        );
    }
    assert_eq!(
        backend
            .open_client("configured", &first.get_cuid())
            .unwrap()
            .as_ref(),
        &limits
    );
    assert_eq!(backend.collections.read()["configured"].clients.len(), 2);

    assert_eq!(
        default.get_collection_info(),
        Some(&CollectionInfo {
            max_state_bytes: 0,
            max_key_bytes: 0,
            max_value_bytes: 0,
            forbidden_keys: Vec::new(),
            max_snapshot_bytes: 10_000_000,
            max_transmission_bytes: 4_000_000,
        })
    );
    let standalone = Client::builder("configured", "standalone")
        .build_and_open()
        .unwrap();
    assert_eq!(
        standalone.get_collection_info(),
        Some(&CollectionInfo::default())
    );
    for collection in ["configured", "unconfigured"] {
        let error = backend
            .create_collection(collection, CollectionInfo::default())
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("[E104] collection already exists: {collection}")
        );
    }
    let fourth = client_builder("configured", "fourth")
        .build_and_open()
        .unwrap();
    assert_eq!(fourth.get_collection_info(), Some(&limits));
}

#[test]
fn can_validate_collection_limit_boundaries_before_creation() {
    let backend = local_connectivity();
    let limits = CollectionInfo {
        max_snapshot_bytes: 1_000,
        max_transmission_bytes: 1_000,
        max_state_bytes: 10_000,
        max_key_bytes: 10,
        max_value_bytes: 10,
        ..CollectionInfo::default()
    };

    for (invalid, expected) in [
        (
            CollectionInfo {
                max_snapshot_bytes: 999,
                ..limits.clone()
            },
            "max_snapshot_bytes must be at least 1000",
        ),
        (
            CollectionInfo {
                max_transmission_bytes: 999,
                ..limits.clone()
            },
            "max_transmission_bytes must be at least 1000",
        ),
        (
            CollectionInfo {
                max_state_bytes: 9_999,
                ..limits.clone()
            },
            "max_state_bytes must be zero or at least 10000",
        ),
        (
            CollectionInfo {
                max_key_bytes: 9,
                ..limits.clone()
            },
            "max_key_bytes must be zero or at least 10",
        ),
        (
            CollectionInfo {
                max_value_bytes: 9,
                ..limits.clone()
            },
            "max_value_bytes must be zero or at least 10",
        ),
    ] {
        let error = backend.create_collection("limits", invalid).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("[E103] invalid collection limits: {expected}")
        );
    }
    assert!(matches!(
        backend.create_collection("", limits.clone()),
        Err(ClientError::InvalidCollectionName(_))
    ));
    backend.create_collection("limits", limits).unwrap();
    backend
        .create_collection(
            "large_snapshot_limit",
            CollectionInfo {
                max_snapshot_bytes: usize::MAX,
                ..CollectionInfo::default()
            },
        )
        .unwrap();
}

struct DropSignal(mpsc::Sender<()>);

impl Drop for DropSignal {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

#[rstest::rstest]
#[case::registered(false)]
#[case::subscribed(true)]
fn can_release_local_client_ownership_without_unsubscribing(#[case] sync: bool) {
    let connectivity = local_connectivity();
    connectivity.set_realtime(false);
    let (collection, key, resource_id) = get_test_ids!();
    let client = Client::builder(collection, "drop-only")
        .with_connectivity(connectivity.clone())
        .build_and_open()
        .unwrap();
    let counter = client.create_datatype(key).build_counter().unwrap();
    if sync {
        counter.increase_by(7).unwrap();
        counter.sync().unwrap();
    }
    let wired = connectivity
        .get_local_datatype_server(&resource_id)
        .unwrap()
        .read()
        .get_wired_datatype(&client.get_cuid())
        .unwrap();
    let weak_wired = Arc::downgrade(&wired);
    let weak_common = Arc::downgrade(&wired.core.attr.client_common);
    let weak_backend = Arc::downgrade(&connectivity);
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let lifetime = DropSignal(dropped_tx);
    wired.get_wired_interceptor().set_before_push(move |_| {
        let _keep_alive = &lifetime;
    });
    drop(wired);
    drop(client);
    drop(counter);
    // Stop acknowledgement precedes loop exit. Wait for wired destruction, not a sleep.
    dropped_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(weak_wired.upgrade().is_none());
    assert!(weak_common.upgrade().is_none());
    // Backend history can outlive clients without retaining their runtime owners.
    assert!(
        connectivity
            .get_local_datatype_server(&resource_id)
            .is_some()
    );
    drop(connectivity);
    // The event loop can still be dropping its own backend reference.
    awaitility::at_most(Duration::from_secs(5)).until(|| weak_backend.upgrade().is_none());
}

#[test]
fn can_keep_an_in_flight_exchange_alive_until_stop_is_processed() {
    let connectivity = local_connectivity();
    connectivity.set_realtime(false);
    let (collection, key, resource_id) = get_test_ids!();
    let client = Client::builder(collection, "in-flight-drop")
        .with_connectivity(connectivity.clone())
        .build_and_open()
        .unwrap();
    let counter = client.create_datatype(key).build_counter().unwrap();
    counter.sync().unwrap();
    let wired = connectivity
        .get_local_datatype_server(&resource_id)
        .unwrap()
        .read()
        .get_wired_datatype(&client.get_cuid())
        .unwrap();
    let weak_wired = Arc::downgrade(&wired);
    let weak_common = Arc::downgrade(&wired.core.attr.client_common);
    let (entered_tx, entered_rx) = crossbeam_channel::bounded(1);
    let (release_tx, release_rx) = crossbeam_channel::bounded(1);
    let (dropped_tx, dropped_rx) = mpsc::channel();
    let lifetime = DropSignal(dropped_tx);
    wired.get_wired_interceptor().set_before_push(move |_| {
        let _keep_alive = &lifetime;
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    drop(wired);
    connectivity.set_realtime(true);
    counter.increase().unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(client);
    // Drop's bounded stop wait can finish while the exchange is blocked.
    drop(counter);
    assert!(weak_wired.upgrade().is_some());
    assert!(dropped_rx.try_recv().is_err());
    release_tx.send(()).unwrap();
    dropped_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(weak_wired.upgrade().is_none());
    assert!(weak_common.upgrade().is_none());
}

#[test]
#[instrument]
fn can_compare_manual_and_realtime_local_connectivity() {
    let lc_manual = local_connectivity();
    lc_manual.set_realtime(false);
    let client_manual = Client::builder(get_test_collection_name!(), "manual client")
        .with_connectivity(lc_manual)
        .build_and_open()
        .unwrap();
    let counter_manual = client_manual
        .create_datatype("manual")
        .build_counter()
        .unwrap();

    let lc_realtime = local_connectivity();
    let client_realtime = Client::builder(get_test_collection_name!(), "realtime client")
        .with_connectivity(lc_realtime)
        .build_and_open()
        .unwrap();
    let counter_realtime = client_realtime
        .create_datatype("realtime")
        .build_counter()
        .unwrap();

    awaitility::at_most(Duration::from_secs(1))
        .poll_interval(Duration::from_micros(100))
        .until(|| counter_realtime.get_state() == DatatypeState::Subscribed);
    assert_ne!(counter_realtime.get_state(), counter_manual.get_state());

    counter_manual.sync().unwrap();
    assert_eq!(counter_manual.get_state(), DatatypeState::Subscribed);
}

#[test]
#[instrument]
fn can_notify_other_clients_after_realtime_push() {
    let connectivity = local_connectivity();
    let (collection, key, _) = get_test_ids!();
    let client1 = Client::builder(collection.clone(), "client1")
        .with_connectivity(connectivity.clone())
        .build_and_open()
        .unwrap();
    let client2 = Client::builder(collection, "client2")
        .with_connectivity(connectivity)
        .build_and_open()
        .unwrap();

    let counter1 = client1
        .create_datatype(key.clone())
        .build_counter()
        .unwrap();
    awaitility::at_most(Duration::from_secs(1))
        .poll_interval(Duration::from_micros(100))
        .until(|| counter1.get_state() == DatatypeState::Subscribed);

    let counter2 = client2.subscribe_datatype(key).build_counter().unwrap();
    awaitility::at_most(Duration::from_secs(1))
        .poll_interval(Duration::from_micros(100))
        .until(|| counter2.get_state() == DatatypeState::Subscribed);

    counter1.increase_by(7).unwrap();

    awaitility::at_most(Duration::from_secs(1))
        .poll_interval(Duration::from_micros(100))
        .until(|| counter2.get_value() == 7);
    assert_eq!(counter1.get_server_version(), counter2.get_server_version());
}

#[test]
#[instrument]
fn does_not_notify_other_clients_in_manual_mode() {
    let connectivity = local_connectivity();
    connectivity.set_realtime(false);
    let (collection, key, _) = get_test_ids!();
    let client1 = Client::builder(collection.clone(), "client1")
        .with_connectivity(connectivity.clone())
        .build_and_open()
        .unwrap();
    let client2 = Client::builder(collection, "client2")
        .with_connectivity(connectivity)
        .build_and_open()
        .unwrap();

    let counter1 = client1
        .create_datatype(key.clone())
        .build_counter()
        .unwrap();
    counter1.sync().unwrap();

    let counter2 = client2.subscribe_datatype(key).build_counter().unwrap();
    counter2.sync().unwrap();

    counter1.increase_by(7).unwrap();
    counter1.sync().unwrap();

    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(counter2.get_value(), 0);

    counter2.sync().unwrap();
    assert_eq!(counter2.get_value(), 7);
}

#[test]
#[instrument]
fn can_keep_serving_snapshots_after_the_creator_unsubscribes() {
    let connectivity = local_connectivity();
    connectivity.set_realtime(false);
    let (collection, key, _) = get_test_ids!();
    let client1 = Client::builder(collection.clone(), "client1")
        .with_connectivity(connectivity.clone())
        .build_and_open()
        .unwrap();
    let client2 = Client::builder(collection.clone(), "client2")
        .with_connectivity(connectivity.clone())
        .build_and_open()
        .unwrap();
    let client3 = Client::builder(collection, "client3")
        .with_connectivity(connectivity.clone())
        .build_and_open()
        .unwrap();

    let counter1 = client1
        .create_datatype(key.clone())
        .build_counter()
        .unwrap();
    counter1.increase_by(7).unwrap();
    counter1.sync().unwrap();

    let counter2 = client2
        .subscribe_datatype(key.clone())
        .build_counter()
        .unwrap();
    counter2.sync().unwrap();
    assert_eq!(counter2.get_value(), 7);

    counter1.unsubscribe().unwrap();
    counter1.sync().unwrap();
    assert_eq!(counter1.get_state(), DatatypeState::Disabled);

    counter2.increase_by(5).unwrap();
    counter2.sync().unwrap();

    let counter3 = client3.subscribe_datatype(key).build_counter().unwrap();
    counter3.sync().unwrap();
    assert_eq!(counter3.get_value(), 12);
}

#[cfg(feature = "snapshot-zstd")]
#[test]
fn can_write_snapshots_with_collection_options() {
    use crate::{
        DataType, SnapshotCompression, SnapshotWriteOptions,
        datatypes::{Crdt, datatype::DatatypeBlanket},
        operations::body::OperationBody,
        types::{push_pull_pack::PushPullPack, uid::Cuid},
    };

    let backend = local_connectivity();
    backend.set_realtime(false);
    let default_options = backend.default_snapshot_write_options;
    let plain_options = SnapshotWriteOptions {
        compression: SnapshotCompression::None,
        ..default_options
    };
    backend
        .create_collection_with_options(
            "plain",
            CollectionInfo::default(),
            crate::LocalCollectionOptions {
                snapshot_write_options: Some(plain_options),
                ..Default::default()
            },
        )
        .unwrap();

    for (collection, options) in [("plain", plain_options), ("default", default_options)] {
        let client = Client::builder(collection, "writer")
            .with_connectivity(backend.clone())
            .build_and_open()
            .unwrap();
        assert_eq!(
            client.get_collection_info(),
            Some(&CollectionInfo::default())
        );
        let counter = client.create_datatype("counter").build_counter().unwrap();
        counter.sync().unwrap();

        let mut request =
            PushPullPack::new(&counter.get_core().core.attr, DatatypeState::Subscribing);
        request.cuid = Cuid::new();
        backend.open_client(collection, &request.cuid).unwrap();
        let response = backend.push_pull(&request).unwrap();
        let snapshot = response.snapshot_transaction.unwrap();
        let OperationBody::Snapshot(body) = &snapshot.operations[0].body else {
            panic!("expected snapshot");
        };
        assert_eq!(
            body.data,
            Crdt::new(DataType::Counter).serialize(options).unwrap()
        );
    }
}

#[rstest::rstest]
#[case::default(true, true, Some("forbidden"))]
#[case::skip_write_policy(false, true, Some("max_state_bytes"))]
#[case::skip_state_size(true, false, Some("forbidden"))]
#[case::skip_both(false, false, None)]
fn can_configure_server_collection_checks_independently(
    #[case] validate_writes: bool,
    #[case] validate_state_size: bool,
    #[case] expected_policy_error: Option<&str>,
) {
    use crate::{
        DatatypeError, LocalCollectionOptions,
        datatypes::datatype::DatatypeBlanket,
        errors::push_pull::PushPullError,
        operations::{Operation, transaction::Transaction},
        types::push_pull_pack::PushPullPack,
    };

    let backend = local_connectivity();
    backend.set_realtime(false);
    backend
        .create_collection_with_options(
            "configured",
            CollectionInfo {
                forbidden_keys: vec!["forbidden".into()],
                max_state_bytes: 10_000,
                ..Default::default()
            },
            LocalCollectionOptions {
                validate_writes,
                validate_state_size,
                ..Default::default()
            },
        )
        .unwrap();
    let client = Client::builder("configured", "writer")
        .with_connectivity(backend.clone())
        .build_and_open()
        .unwrap();
    let map = client.create_datatype("map").build_map().unwrap();
    assert!(matches!(
        map.put("forbidden", &1),
        Err(DatatypeError::CollectionPolicyViolation(_))
    ));

    let mut request = PushPullPack::new(&map.get_core().core.attr, DatatypeState::Creating);
    let tx_with = |mut op: Operation| {
        op.set_lamport(1);
        let mut tx = Transaction::new(&client.get_cuid(), 1);
        tx.push_operation(op);
        vec![Arc::new(tx)]
    };
    request.transactions = tx_with(Operation::new_counter_increase(1));
    let malformed = backend.push_pull(&request).unwrap();
    assert!(
        matches!(malformed.error, Some(PushPullError::ProtocolViolation(_))),
        "{:?}",
        malformed.error
    );

    let value = serde_json::to_vec(&"x".repeat(10_000))
        .unwrap()
        .into_boxed_slice();
    request.transactions = tx_with(Operation::new_map_put("forbidden".into(), value.clone()));
    let response = backend.push_pull(&request).unwrap();
    if let Some(detail) = expected_policy_error {
        assert!(matches!(
            response.error,
            Some(PushPullError::CollectionPolicyViolation(message)) if message.contains(detail)
        ));
    } else {
        assert!(response.error.is_none(), "{:?}", response.error);
        assert_eq!(response.checkpoint.cseq, 1);
        return;
    }

    // A valid key isolates the merged size check at the same client sequence.
    request.transactions = tx_with(Operation::new_map_put("allowed".into(), value));
    let response = backend.push_pull(&request).unwrap();
    if validate_state_size {
        assert!(matches!(
            response.error,
            Some(PushPullError::CollectionPolicyViolation(message)) if message.contains("max_state_bytes")
        ));
    } else {
        assert!(response.error.is_none(), "{:?}", response.error);
        assert_eq!(response.checkpoint.cseq, 1);
    }
}

#[test]
fn can_preserve_client_generation_bounds_across_resource_recreation() {
    use crate::{
        DataType,
        connectivity::Connectivity,
        datatypes::{
            common::new_attribute, wired::WiredDatatype, wired_interceptor::WiredInterceptor,
        },
        errors::push_pull::PushPullError,
        types::push_pull_pack::PushPullPack,
    };
    let connectivity = local_connectivity();
    connectivity.set_realtime(false);
    let attr = new_attribute!(DataType::Counter);
    connectivity
        .open_client(&attr.client_common.collection, &attr.client_common.cuid)
        .unwrap();
    let wired = WiredDatatype::new_arc_for_test(
        attr.clone(),
        DatatypeState::Creating,
        WiredInterceptor::new_arc(),
    );
    let (sender, _receiver) = crossbeam_channel::unbounded();
    connectivity.register(wired.clone(), sender.clone());
    let mut old = PushPullPack::new(&attr, DatatypeState::Creating);
    let first_generation = old.sub_gen;
    assert!(connectivity.push_pull(&old).unwrap().error.is_none());
    assert_eq!(
        connectivity
            .get_local_datatype_server(&attr.resource_id())
            .unwrap()
            .read()
            .max_sub_gens
            .get(&old.cuid)
            .copied(),
        Some(first_generation)
    );
    let mut peer = PushPullPack::new(&attr, DatatypeState::Subscribing);
    peer.cuid = crate::types::uid::Cuid::new();
    assert!(connectivity.push_pull(&peer).unwrap().error.is_none());
    old.state = DatatypeState::Unsubscribing;
    assert!(connectivity.push_pull(&old).unwrap().error.is_none());
    peer.state = DatatypeState::Unsubscribing;
    assert!(connectivity.push_pull(&peer).unwrap().error.is_none());
    assert!(
        connectivity
            .get_local_datatype_server(&attr.resource_id())
            .is_none()
    );
    let repeated = connectivity.push_pull(&old).unwrap();
    assert!(repeated.error.is_none());
    assert_eq!(repeated.state, DatatypeState::Disabled);
    old.state = DatatypeState::Subscribing;
    assert!(matches!(
        connectivity.push_pull(&old).unwrap().error,
        Some(PushPullError::MissingSubscription(_))
    ));
    old.state = DatatypeState::Unsubscribing;

    for _ in 0..2 {
        connectivity.register(wired.clone(), sender.clone());
        peer.state = DatatypeState::Subscribing;
        assert!(matches!(
            connectivity.push_pull(&peer).unwrap().error,
            Some(PushPullError::MissingSubscription(_))
        ));
        // A delayed unsubscribe must leave this newly registered handle intact.
        assert!(connectivity.push_pull(&old).unwrap().error.is_none());
        assert!(
            connectivity
                .get_local_datatype_server(&attr.resource_id())
                .is_some()
        );
        old.state = DatatypeState::Creating;
        assert!(matches!(
            connectivity.push_pull(&old).unwrap().error,
            Some(PushPullError::MissingSubscription(_))
        ));
        let mut fresh = PushPullPack::new(&attr, DatatypeState::Creating);
        assert!(connectivity.push_pull(&fresh).unwrap().error.is_none());
        fresh.state = DatatypeState::Unsubscribing;
        assert!(connectivity.push_pull(&fresh).unwrap().error.is_none());
        assert!(
            connectivity
                .get_local_datatype_server(&attr.resource_id())
                .is_none()
        );
        old = fresh;
    }
    assert_eq!(
        connectivity.datatype_servers.read().retired_sub_gens[&attr.resource_id()]
            .get(&old.cuid)
            .copied(),
        Some(old.sub_gen)
    );
    // The same CUID may still use an older generation on a different resource.
    assert!(first_generation < old.sub_gen);
    old.sub_gen = first_generation;
    old.key = "independent-resource".into();
    old.state = DatatypeState::Creating;
    let mut other_attr = new_attribute!(DataType::Counter);
    let other_attr_mut = Arc::get_mut(&mut other_attr).unwrap();
    other_attr_mut.key = old.key.clone();
    other_attr_mut.client_common = attr.client_common.clone();
    let other = WiredDatatype::new_arc_for_test(
        other_attr,
        DatatypeState::Creating,
        WiredInterceptor::new_arc(),
    );
    connectivity.register(other, sender);
    assert!(connectivity.push_pull(&old).unwrap().error.is_none());
    assert_eq!(
        connectivity
            .get_local_datatype_server(&old.resource_id())
            .unwrap()
            .read()
            .max_sub_gens
            .get(&old.cuid)
            .copied(),
        Some(first_generation)
    );
}

#[test]
#[instrument]
fn can_remove_server_when_last_client_unsubscribes() {
    let connectivity = local_connectivity();
    connectivity.set_realtime(false);
    let (collection, key, resource_id) = get_test_ids!();
    let client1 = Client::builder(collection.clone(), "client1")
        .with_connectivity(connectivity.clone())
        .build_and_open()
        .unwrap();
    let client2 = Client::builder(collection, "client2")
        .with_connectivity(connectivity.clone())
        .build_and_open()
        .unwrap();

    let counter1 = client1
        .create_datatype(key.clone())
        .build_counter()
        .unwrap();
    counter1.increase_by(7).unwrap();
    counter1.sync().unwrap();

    counter1.unsubscribe().unwrap();
    counter1.sync().unwrap();
    assert!(
        connectivity
            .get_local_datatype_server(&resource_id)
            .is_none()
    );

    let counter2 = client2.create_datatype(key).build_counter().unwrap();
    counter2.sync().unwrap();
    assert_eq!(counter2.get_value(), 0);
    assert_eq!(counter2.get_state(), DatatypeState::Subscribed);
}
