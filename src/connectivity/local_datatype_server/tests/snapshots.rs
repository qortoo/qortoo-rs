use super::*;

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
    let mut server = LocalDatatypeServer::new(&attr, Default::default());
    let create = PushPullPack::new(&attr, DatatypeState::Creating);
    assert!(server.process(&create, false).unwrap().error.is_none());
    let subscribe = PushPullPack::with_sub_gen(&attr, DatatypeState::Subscribing, create.sub_gen);
    let pulled = server.process(&subscribe, false).unwrap();
    let snapshot = pulled.snapshot_transaction.unwrap();
    assert_eq!(snapshot.sseq, 0);
    assert_eq!(snapshot.operations[0].lamport, 0);
    assert_eq!(snapshot.cuid, crate::types::uid::Cuid::new_nil());
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
    let mut server = LocalDatatypeServer::new(&attr, Default::default());
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
    subscribe.cuid = crate::types::uid::Cuid::new();
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
    let crate::operations::body::OperationBody::Snapshot(body) = &snapshot.operations[0].body
    else {
        panic!("expected snapshot");
    };
    let mut restored = crate::datatypes::Crdt::new(DataType::Counter);
    restored.deserialize(&body.data).unwrap();
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
