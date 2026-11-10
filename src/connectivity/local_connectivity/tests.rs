use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

use tracing::instrument;

use crate::{
    Client, Datatype, DatatypeState,
    connectivity::local_connectivity::LocalConnectivity,
    utils::test_utils::{get_test_collection_name, get_test_func_name, get_test_ids},
};

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
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let (collection, key, resource_id) = get_test_ids!();
    let client = Client::builder(collection, "drop-only")
        .with_connectivity(connectivity.clone())
        .build()
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
    assert!(weak_backend.upgrade().is_none());
}

#[test]
fn can_keep_an_in_flight_exchange_alive_until_stop_is_processed() {
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let (collection, key, resource_id) = get_test_ids!();
    let client = Client::builder(collection, "in-flight-drop")
        .with_connectivity(connectivity.clone())
        .build()
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
    let lc_manual = LocalConnectivity::new_arc();
    lc_manual.set_realtime(false);
    let client_manual = Client::builder(get_test_collection_name!(), "manual client")
        .with_connectivity(lc_manual)
        .build()
        .unwrap();
    let counter_manual = client_manual
        .create_datatype("manual")
        .build_counter()
        .unwrap();

    let lc_realtime = LocalConnectivity::new_arc();
    let client_realtime = Client::builder(get_test_collection_name!(), "realtime client")
        .with_connectivity(lc_realtime)
        .build()
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
    let connectivity = LocalConnectivity::new_arc();
    let (collection, key, _) = get_test_ids!();
    let client1 = Client::builder(collection.clone(), "client1")
        .with_connectivity(connectivity.clone())
        .build()
        .unwrap();
    let client2 = Client::builder(collection, "client2")
        .with_connectivity(connectivity)
        .build()
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
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let (collection, key, _) = get_test_ids!();
    let client1 = Client::builder(collection.clone(), "client1")
        .with_connectivity(connectivity.clone())
        .build()
        .unwrap();
    let client2 = Client::builder(collection, "client2")
        .with_connectivity(connectivity)
        .build()
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
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let (collection, key, _) = get_test_ids!();
    let client1 = Client::builder(collection.clone(), "client1")
        .with_connectivity(connectivity.clone())
        .build()
        .unwrap();
    let client2 = Client::builder(collection.clone(), "client2")
        .with_connectivity(connectivity.clone())
        .build()
        .unwrap();
    let client3 = Client::builder(collection, "client3")
        .with_connectivity(connectivity.clone())
        .build()
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
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let attr = new_attribute!(DataType::Counter);
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
    let connectivity = LocalConnectivity::new_arc();
    connectivity.set_realtime(false);
    let (collection, key, resource_id) = get_test_ids!();
    let client1 = Client::builder(collection.clone(), "client1")
        .with_connectivity(connectivity.clone())
        .build()
        .unwrap();
    let client2 = Client::builder(collection, "client2")
        .with_connectivity(connectivity.clone())
        .build()
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
