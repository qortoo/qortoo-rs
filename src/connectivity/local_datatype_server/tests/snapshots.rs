use std::sync::atomic::{AtomicUsize, Ordering};

use super::*;
use crate::{
    SnapshotDecodeLimits, SnapshotErrorReason, datatypes::Crdt, operations::body::OperationBody,
};

#[test]
fn can_discard_transactions_committed_while_a_subscription_snapshot_is_in_flight() {
    let f = TestFixture::new(get_test_ids!());
    let creator = f.client("creator");
    let source = f.create_counter(&creator);
    source.increase_by(10).unwrap();
    source.sync().unwrap();

    let client = f.client("subscriber");
    let replica = client
        .subscribe_or_create_datatype(f.key.clone())
        .build_counter()
        .unwrap();
    let server = f.server();
    let wired = server
        .read()
        .get_wired_datatype(&client.get_cuid())
        .unwrap();
    let interceptor = wired.get_wired_interceptor();
    let writing_replica = replica.clone();
    let weak_wired = Arc::downgrade(&wired);
    interceptor.set_after_pull(move |pull| {
        assert!(pull.error.is_none());
        assert!(pull.snapshot_transaction.is_some());
        assert_eq!(
            writing_replica.get_state(),
            DatatypeState::SubscribingOrCreating
        );
        // The request was empty, but a local write can finish before snapshot application.
        writing_replica.increase_by(5).unwrap();
        assert_eq!(writing_replica.get_value(), 5);
        assert_eq!(
            weak_wired
                .upgrade()
                .unwrap()
                .core
                .mutable
                .read()
                .push_buffer
                .iter()
                .count(),
            1
        );
        Ok(())
    });
    let result = replica.sync();
    interceptor.set_after_pull(|_| Ok(()));
    result.unwrap();

    assert_eq!(replica.get_state(), DatatypeState::Subscribed);
    assert_eq!(replica.get_value(), 10);
    assert_eq!(wired.core.mutable.read().push_buffer.iter().count(), 0);
    interceptor.set_before_push(|push| assert!(push.transactions.is_empty()));
    replica.sync().unwrap();
    source.sync().unwrap();
    assert_eq!(replica.get_value(), 10);
    assert_eq!(source.get_value(), 10);
    let backend = server.read();
    assert_eq!(backend.synced.sseq_at_server, 1);
    assert_eq!(
        backend.synced.checkpoints[&client.get_cuid()].cseq_at_server,
        0
    );
    assert_eq!(backend.synced.crdt.as_counter().unwrap().value(), 10);
}

#[test]
fn can_bootstrap_and_retry_a_snapshot_after_history_is_pruned() {
    let f = TestFixture::new(get_test_ids!());
    let creator = f.client("creator");
    let source = f.create_counter(&creator);
    source.increase_by(7).unwrap();
    source.sync().unwrap();
    source.sync().unwrap();
    let server = f.server();
    assert_eq!(server.read().synced.sseq_history_pruned, 1);
    assert!(server.read().synced.history.is_empty());
    source.increase_by(3).unwrap();
    source.sync().unwrap();

    let reader = f.client("reader");
    let replica = f.subscribe_counter(&reader);
    let interceptor = f.interceptor(&reader);
    interceptor
        .set_after_pull(|_| Err(DatatypeError::SyncFailed("response lost".into()).mapping()));
    assert!(replica.sync().is_err());
    assert_eq!(replica.get_server_version(), 0);
    assert_eq!(
        server.read().synced.checkpoints[&reader.get_cuid()].sseq_at_client,
        1
    );
    assert_eq!(server.read().synced.sseq_history_pruned, 1);
    assert_eq!(server.read().synced.history.len(), 1);
    interceptor.set_after_pull(|_| Ok(()));
    replica.sync().unwrap();
    assert_eq!(replica.get_value(), 10);
    assert_eq!(replica.get_server_version(), 2);
    assert_eq!(
        server.read().synced.checkpoints[&reader.get_cuid()].sseq_at_client,
        1
    );
    source.sync().unwrap();
    replica.sync().unwrap();
    assert_eq!(server.read().synced.sseq_history_pruned, 2);
    assert!(server.read().synced.history.is_empty());
    replica.increase_by(2).unwrap();
    replica.sync().unwrap();
    source.sync().unwrap();
    assert_eq!(source.get_value(), 12);
    assert_eq!(replica.get_value(), 12);
}

#[test]
fn can_keep_a_failed_snapshot_application_unacknowledged() {
    let f = TestFixture::new(get_test_ids!());
    let creator = f.client("creator");
    let reader = f.client("reader");
    let source = f.create_counter(&creator);
    source.increase().unwrap();
    source.sync().unwrap();
    let replica = f.subscribe_counter_readonly(&reader);
    let interceptor = f.interceptor(&reader);
    interceptor.set_after_pull(|pull| {
        // Force the actual snapshot application to fail after the backend supplied it.
        let snapshot = Arc::make_mut(pull.snapshot_transaction.as_mut().unwrap());
        snapshot.operations.clear();
        Ok(())
    });
    assert!(replica.sync().is_err());
    let server = f.server();
    assert_eq!(
        server.read().synced.checkpoints[&reader.get_cuid()].sseq_at_client,
        0
    );
    assert_eq!(server.read().synced.sseq_history_prunable(), 0);
    assert_eq!(server.read().synced.sseq_history_pruned, 0);
    assert_eq!(server.read().synced.history.len(), 1);
    assert_eq!(replica.get_server_version(), 0);
}

#[test]
fn can_serve_an_empty_backend_snapshot_before_any_writes() {
    let attr = new_attribute!(DataType::Counter);
    let mut server = test_server(&attr);
    let create = PushPullPack::new(&attr, DatatypeState::Creating);
    assert!(server.process(&create, false).unwrap().error.is_none());
    let subscribe = PushPullPack::with_sub_gen(&attr, DatatypeState::Subscribing, create.sub_gen);
    let pulled = server.process(&subscribe, false).unwrap();
    let snapshot = pulled.snapshot_transaction.unwrap();
    assert_eq!(snapshot.sseq, 0);
    assert_eq!(snapshot.operations[0].lamport, 0);
    assert_eq!(snapshot.cuid, Cuid::new_nil());
    assert!(pulled.transactions.is_empty());
}

#[rstest]
#[case::expired_handle(false)]
#[case::busy_transaction(true)]
fn can_serve_a_snapshot_without_accessing_client_state(#[case] keep_busy_handle: bool) {
    use crate::datatypes::{wired::WiredDatatype, wired_interceptor::WiredInterceptor};
    let attr = new_attribute!(DataType::Counter);
    let wired = WiredDatatype::new_arc_for_test(
        attr.clone(),
        DatatypeState::Subscribed,
        WiredInterceptor::new_arc(),
    );
    let mut server = test_server(&attr);
    let (sender, _) = crossbeam_channel::unbounded();
    server.insert_client_item(wired.clone(), sender);
    let mut create = PushPullPack::new(&attr, DatatypeState::Creating);
    create.transactions = vec![transaction(
        &create.cuid,
        1,
        Operation::new_counter_increase(7),
    )];
    assert!(server.process(&create, false).unwrap().error.is_none());
    let handle = keep_busy_handle.then_some(wired);
    if let Some(wired) = &handle {
        wired
            .core
            .mutable
            .write()
            .execute_local_operation(Operation::new_counter_increase(5))
            .unwrap();
    } else {
        assert!(server.get_wired_datatype(&create.cuid).is_none());
    }
    let gate = handle.as_ref().map(|wired| wired.core.tx_mutex.lock());
    let mut subscribe = PushPullPack::new(&attr, DatatypeState::Subscribing);
    subscribe.cuid = Cuid::new();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        done_tx
            .send(server.process(&subscribe, false).unwrap())
            .unwrap();
    });
    let result = done_rx.recv_timeout(Duration::from_secs(5));
    drop(gate);
    worker.join().unwrap();
    let pulled = result.expect("subscription must not wait on the client's transaction gate");
    let snapshot = pulled.snapshot_transaction.unwrap();
    assert_eq!(snapshot.sseq, 1);
    assert_eq!(snapshot.operations[0].lamport, 1);
    let OperationBody::Snapshot(body) = &snapshot.operations[0].body else {
        panic!("expected snapshot");
    };
    let mut restored = Crdt::new(DataType::Counter);
    restored
        .deserialize(&body.data, Default::default())
        .unwrap();
    assert_eq!(restored.as_counter().unwrap().value(), 7);
    assert!(pulled.transactions.is_empty());
}

#[test]
#[instrument]
fn can_subscribe_from_backend_state_after_clients_are_removed() {
    let f = TestFixture::new(get_test_ids!());

    // The backend retains accepted state independently of client registrations.
    let client1 = f.client("creator");
    let counter1 = f.create_counter(&client1);
    counter1.increase_by(7).unwrap();
    counter1.sync().unwrap();
    assert_eq!(counter1.get_state(), DatatypeState::Subscribed);

    // A second subscriber receives the accepted state.
    let client2 = f.client("subscriber");
    let counter2 = f.subscribe_counter(&client2);
    counter2.sync().unwrap();
    assert_eq!(counter2.get_state(), DatatypeState::Subscribed);

    f.remove_client_subscription(&client1);
    f.remove_client_subscription(&client2);

    let client3 = f.client("new-subscriber");
    let counter3 = f.subscribe_counter(&client3);
    counter3.sync().unwrap();
    assert_eq!(counter3.get_state(), DatatypeState::Subscribed);
    assert_eq!(counter3.get_value(), 7);
}

#[test]
fn can_apply_snapshot_policy_for_each_available_writer_and_datatype() {
    use crate::{
        SnapshotCodecKind as Codec, SnapshotCompression as Compression, SnapshotWriteOptions,
    };
    for codec in [1, 2].into_iter().filter_map(Codec::from_id) {
        for compression in [0, 1].into_iter().filter_map(Compression::from_id) {
            let options = SnapshotWriteOptions { codec, compression };
            for kind in [DataType::Counter, DataType::Variable, DataType::Map] {
                let attr = new_attribute!(kind);
                let mut server =
                    LocalDatatypeServer::new(&attr, Default::default(), collection_config(options));
                let mut create = PushPullPack::new(&attr, DatatypeState::Creating);
                let operation = match kind {
                    DataType::Counter => Operation::new_counter_increase(7),
                    DataType::Variable => Operation::new_variable_set(Box::from(b"[7]".as_slice())),
                    DataType::Map => {
                        Operation::new_map_put("key".into(), Box::from(b"null".as_slice()))
                    }
                };
                create.transactions = vec![transaction(&create.cuid, 1, operation)];
                assert!(server.process(&create, false).unwrap().error.is_none());
                let subscribe =
                    PushPullPack::with_sub_gen(&attr, DatatypeState::Subscribing, create.sub_gen);
                let pulled = server.process(&subscribe, false).unwrap();
                assert!(pulled.error.is_none());
                let snapshot = pulled.snapshot_transaction.unwrap();
                let OperationBody::Snapshot(body) = &snapshot.operations[0].body else {
                    panic!("expected snapshot")
                };
                assert_eq!(&body.data[..4], b"QTSN");
                assert_eq!(body.data[6], codec.id());
                assert_eq!(body.data[7], compression.id());
                let mut restored = Crdt::new(kind);
                restored
                    .deserialize(&body.data, Default::default())
                    .unwrap();
                match kind {
                    DataType::Counter => assert_eq!(restored.as_counter().unwrap().value(), 7),
                    DataType::Variable => {
                        assert_eq!(restored.as_variable().unwrap().value(), b"[7]")
                    }
                    DataType::Map => assert_eq!(
                        restored.as_map().unwrap().shared_value("key").as_deref(),
                        Some(b"null".as_slice())
                    ),
                }
            }
        }
    }
}

#[test]
fn can_keep_subscription_progress_when_snapshot_capture_fails() {
    use crate::types::operation_context::OperationContext;
    let attr = new_attribute!(DataType::Variable);
    let mut server = test_server(&attr);
    let mut create = PushPullPack::new(&attr, DatatypeState::Creating);
    create.transactions = vec![transaction(
        &create.cuid,
        1,
        Operation::new_variable_set(Box::from(b"null".as_slice())),
    )];
    assert!(server.process(&create, false).unwrap().error.is_none());
    let previous = server.synced.checkpoints[&create.cuid].clone();
    let history_len = server.synced.history.len();
    let pruned = server.synced.sseq_history_pruned;
    let gc_safe = server.synced.sseq_gc_safe;
    let mut invalid = Crdt::new(DataType::Variable);
    let mut operation = Operation::new_variable_set(Box::from(b"not-json".as_slice()));
    operation.set_lamport(1);
    invalid
        .execute_remote_operation(&OperationContext::try_new(&operation, &create.cuid).unwrap())
        .unwrap();
    let valid = std::mem::replace(&mut server.synced.crdt, invalid);
    let subscribe =
        PushPullPack::with_sub_gen(&attr, DatatypeState::Subscribing, create.sub_gen + 1);
    let pulled = server.process(&subscribe, false).unwrap();
    assert!(matches!(
        pulled.error,
        Some(PushPullError::SnapshotUnavailable(_))
    ));
    assert!(pulled.snapshot_transaction.is_none());
    assert_eq!(server.synced.checkpoints[&create.cuid], previous);
    assert_eq!(server.max_sub_gens[&create.cuid], create.sub_gen);
    assert_eq!(server.synced.history.len(), history_len);
    assert_eq!(server.synced.sseq_history_pruned, pruned);
    assert_eq!(server.synced.sseq_gc_safe, gc_safe);
    server.synced.crdt = valid;
    let pulled = server.process(&subscribe, false).unwrap();
    assert!(pulled.error.is_none());
    assert!(pulled.snapshot_transaction.is_some());
    assert_eq!(server.max_sub_gens[&create.cuid], subscribe.sub_gen);
}

#[rstest]
#[case::input(SnapshotDecodeLimits { max_input_bytes: 0, ..Default::default() })]
#[case::raw(SnapshotDecodeLimits { max_raw_bytes: 0, ..Default::default() })]
fn can_disable_after_a_snapshot_limit_failure_without_installing_state(
    #[case] limits: SnapshotDecodeLimits,
) {
    let f = TestFixture::new(get_test_ids!());
    let creator = f.client("creator");
    let source = f.create_counter(&creator);
    source.increase_by(7).unwrap();
    source.sync().unwrap();
    let reader = Client::builder(f.collection.clone(), "reader")
        .with_connectivity(f.connectivity.clone())
        .with_snapshot_decode_limits(limits)
        .build_and_open()
        .unwrap();
    let replica = reader
        .subscribe_or_create_datatype(f.key.clone())
        .build_counter()
        .unwrap();
    let wired = f
        .server()
        .read()
        .get_wired_datatype(&reader.get_cuid())
        .unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let writer = replica.clone();
    let previous_id = Arc::new(parking_lot::Mutex::new(None));
    let captured_id = previous_id.clone();
    let captured_wired = Arc::downgrade(&wired);
    f.interceptor(&reader).set_after_pull(move |_| {
        counted.fetch_add(1, Ordering::SeqCst);
        writer.increase_by(3).unwrap();
        *captured_id.lock() = Some(
            captured_wired
                .upgrade()
                .unwrap()
                .core
                .mutable
                .read()
                .op_id
                .clone(),
        );
        Ok(())
    });
    let previous_checkpoint = wired.core.mutable.read().checkpoint;
    let result = replica.sync();
    assert!(
        matches!(
            result,
            Err(DatatypeError::SnapshotDecode(
                SnapshotErrorReason::LimitExceeded
            ))
        ),
        "{result:?}"
    );
    assert_eq!(replica.get_state(), DatatypeState::Disabled);
    assert_eq!(replica.get_value(), 3);
    let mutable = wired.core.mutable.read();
    assert_eq!(&mutable.op_id, previous_id.lock().as_ref().unwrap());
    assert_eq!(mutable.checkpoint, previous_checkpoint);
    assert!(mutable.push_buffer.iter().next().is_none());
    drop(mutable);
    thread::sleep(Duration::from_millis(800));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn can_propagate_snapshot_policy_from_local_connectivity() {
    use crate::{
        SnapshotCodecKind as Codec, SnapshotCompression as Compression, SnapshotWriteOptions,
    };
    for codec in [1, 2].into_iter().filter_map(Codec::from_id) {
        let options = SnapshotWriteOptions {
            codec,
            compression: Compression::None,
        };
        let connectivity = LocalConnectivity::new_arc_with_snapshot_options(options);
        connectivity.set_realtime(false);
        let mut f = TestFixture::new(get_test_ids!());
        f.connectivity = connectivity;
        let creator = f.client("creator");
        let source = f.create_counter(&creator);
        source.increase_by(7).unwrap();
        source.sync().unwrap();
        let reader = f.client("reader");
        let replica = f.subscribe_counter(&reader);
        f.interceptor(&reader).set_after_pull(move |pull| {
            let snapshot = pull.snapshot_transaction.as_ref().unwrap();
            let OperationBody::Snapshot(body) = &snapshot.operations[0].body else {
                panic!("expected snapshot")
            };
            assert_eq!(body.data[6], codec.id());
            assert_eq!(body.data[7], 0);
            Ok(())
        });
        replica.sync().unwrap();
        assert_eq!(replica.get_value(), 7);
    }
}

#[test]
fn can_keep_syncing_when_a_new_subscription_exceeds_the_snapshot_limit() {
    let attr = new_attribute!(DataType::Variable);
    let mut options = snapshot_write_options();
    options.compression = crate::SnapshotCompression::None;
    let config = LocalCollectionConfig::new(
        crate::CollectionInfo {
            max_snapshot_bytes: 1000,
            ..Default::default()
        },
        crate::LocalCollectionOptions {
            validate_state_size: false,
            ..crate::LocalCollectionOptions::new(options)
        },
    );
    let mut server = LocalDatatypeServer::new(&attr, Default::default(), Arc::new(config));
    let mut push = PushPullPack::new(&attr, DatatypeState::Creating);
    let set = |value: &str| Operation::new_variable_set(value.as_bytes().into());
    push.transactions = vec![transaction(&push.cuid, 1, set("1"))];
    assert!(server.process(&push, false).unwrap().error.is_none());

    let mut reader = PushPullPack::new(&attr, DatatypeState::Subscribing);
    reader.cuid = Cuid::new();
    let subscribed = server.process(&reader, false).unwrap();
    assert!(subscribed.snapshot_transaction.is_some());
    reader.state = DatatypeState::Subscribed;
    reader.duid = subscribed.duid;
    reader.checkpoint = subscribed.checkpoint;

    let large = format!("\"{}\"", "x".repeat(2000));
    push.state = DatatypeState::Subscribed;
    push.transactions = vec![
        transaction(&push.cuid, 2, set("2")),
        transaction(&push.cuid, 3, set(&large)),
    ];
    let accepted = server.process(&push, false).unwrap();
    assert!(accepted.error.is_none());
    assert_eq!(accepted.checkpoint, CheckPoint::new(3, 3));

    let mut subscribe = PushPullPack::new(&attr, DatatypeState::Subscribing);
    subscribe.cuid = Cuid::new();
    let history = server.synced.history.clone();
    let checkpoints = server.synced.checkpoints.clone();
    let generations = server.max_sub_gens.clone();
    let rejected = server.process(&subscribe, false).unwrap();
    assert_eq!(
        rejected.error,
        Some(PushPullError::SnapshotUnavailable(
            "snapshot exceeds the collection or default decoder limits".into()
        ))
    );
    assert!(rejected.snapshot_transaction.is_none());
    assert_eq!(
        server.synced.crdt.as_variable().unwrap().value(),
        large.as_bytes()
    );
    assert_eq!(server.synced.history, history);
    assert_eq!(server.synced.checkpoints, checkpoints);
    assert_eq!(server.max_sub_gens, generations);

    let pulled = server.process(&reader, false).unwrap();
    assert!(pulled.error.is_none());
    assert_eq!(pulled.checkpoint.sseq, 3);
    assert_eq!(
        pulled.transactions,
        history
            .into_iter()
            .filter(|tx| tx.sseq > 1)
            .collect::<Vec<_>>()
    );
    assert_eq!(pulled.transactions.len(), 2);

    push.checkpoint = accepted.checkpoint;
    push.transactions = vec![transaction(&push.cuid, 4, set("4"))];
    assert!(server.process(&push, false).unwrap().error.is_none());
    let subscribed = server.process(&subscribe, false).unwrap();
    assert!(subscribed.error.is_none());
    assert_eq!(subscribed.snapshot_transaction.unwrap().sseq, 4);
}
