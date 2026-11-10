use super::*;

#[rstest]
#[case::repeated_missing_subscription(true)]
#[case::missing_snapshot(false)]
fn can_disable_when_subscription_recovery_cannot_complete(#[case] missing_again: bool) {
    let f = TestFixture::new(get_test_ids!());
    let client = f.client("client");
    let counter = f.create_counter(&client);
    counter.increase_by(10).unwrap();
    counter.sync().unwrap();
    let interceptor = f.interceptor(&client);
    let observed = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let requests = observed.clone();
    interceptor.set_before_push(move |push| requests.lock().push(push.state));
    interceptor.set_after_pull(move |pull| {
        if missing_again {
            pull.error = Some(PushPullError::MissingSubscription(
                "subscription lost again".into(),
            ));
            pull.state = DatatypeState::Disabled;
        } else {
            pull.snapshot_transaction = None;
        }
        Ok(())
    });
    f.remove_client_subscription(&client);
    assert!(counter.sync().is_err());
    assert_eq!(counter.get_state(), DatatypeState::Disabled);
    assert_eq!(
        *observed.lock(),
        vec![DatatypeState::Subscribed, DatatypeState::Subscribing]
    );
    assert!(counter.sync().is_err());
    assert_eq!(observed.lock().len(), 2);
}

#[rstest]
#[case::manual(false)]
#[case::realtime(true)]
fn can_resubscribe_after_losing_a_subscription_without_resending_pending_work(
    #[case] realtime: bool,
) {
    let f = TestFixture::new(get_test_ids!());
    let creator = f.client("creator");
    let source = f.create_counter(&creator);
    source.increase_by(10).unwrap();
    source.sync().unwrap();
    let client = f.client("subscriber");
    let replica = f.subscribe_counter(&client);
    replica.sync().unwrap();
    replica.increase_by(2).unwrap();
    replica.sync().unwrap();
    assert_eq!(replica.get_synced_client_version(), 1);
    replica.increase_by(5).unwrap();
    source.increase_by(7).unwrap();
    source.sync().unwrap();
    let server = f.server();
    let wired = server
        .read()
        .get_wired_datatype(&client.get_cuid())
        .unwrap();
    let interceptor = wired.get_wired_interceptor();
    let observed = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let requests = observed.clone();
    interceptor.set_before_push(move |push| {
        if push.state == DatatypeState::Subscribing {
            assert_eq!(push.checkpoint, CheckPoint::default());
            assert!(push.transactions.is_empty());
        }
        requests.lock().push(push.state);
    });
    let writing_replica = replica.clone();
    interceptor.set_after_pull(move |pull| {
        if pull.snapshot_transaction.is_some() {
            assert_eq!(
                writing_replica.get_state(),
                DatatypeState::SubscribingOrCreating
            );
            writing_replica.increase_by(3).unwrap();
        }
        Ok(())
    });
    f.remove_client_subscription(&client);
    f.connectivity.set_realtime(realtime);
    let result = replica.sync();
    interceptor.set_after_pull(|_| Ok(()));
    result.unwrap();
    assert!(
        observed
            .lock()
            .windows(2)
            .any(|states| states == [DatatypeState::Subscribed, DatatypeState::Subscribing])
    );
    assert_eq!(replica.get_state(), DatatypeState::Subscribed);
    assert_eq!(replica.get_value(), 19);
    assert_eq!(replica.get_synced_client_version(), 0);
    assert_eq!(wired.core.mutable.read().push_buffer.iter().count(), 0);
    assert!(client.get_datatype(&f.key).is_some());
    assert!(
        server
            .read()
            .get_wired_datatype(&client.get_cuid())
            .is_some()
    );
    replica.increase_by(4).unwrap();
    replica.sync().unwrap();
    source.sync().unwrap();
    assert_eq!(source.get_value(), 23);
    assert_eq!(replica.get_value(), 23);
    assert_eq!(replica.get_synced_client_version(), 1);
}

#[test]
fn can_restart_client_sequences_after_unsubscribe_and_resubscribe() {
    let f = TestFixture::new(get_test_ids!());
    let client = f.client("rejoining");
    let peer_client = f.client("remaining");
    let counter = f.create_counter(&client);
    counter.increase_by(7).unwrap();
    counter.sync().unwrap();
    let peer = f.subscribe_counter(&peer_client);
    peer.sync().unwrap();
    peer.increase_by(3).unwrap();
    peer.sync().unwrap();
    counter.sync().unwrap();

    counter.increase_by(2).unwrap();
    counter.unsubscribe().unwrap();
    counter.sync().unwrap();
    peer.sync().unwrap();
    assert_eq!(peer.get_value(), 12);
    let server = f.server();
    {
        let server = server.read();
        assert!(!server.synced.checkpoints.contains_key(&client.get_cuid()));
        assert_eq!(
            server
                .synced
                .checkpoints
                .get(&peer_client.get_cuid())
                .map(|cp| cp.cseq_at_server),
            Some(1)
        );
    }

    let renewed = f.subscribe_counter(&client);
    renewed.sync().unwrap();
    assert_eq!(renewed.get_value(), 12);
    assert_eq!(renewed.get_client_version(), 0);
    renewed.increase_by(5).unwrap();
    renewed.sync().unwrap();
    assert_eq!(renewed.get_synced_client_version(), 1);
    peer.sync().unwrap();
    assert_eq!(peer.get_value(), 17);
    assert_eq!(renewed.get_value(), 17);
}

#[test]
fn can_clear_client_progress_when_unsubscribe_rejects_pending_work() {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr, Default::default());
    let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
    pushed.transactions = vec![transaction(
        &pushed.cuid,
        1,
        Operation::new_counter_increase(7),
    )];
    server.process(&pushed, false).unwrap();
    pushed.state = DatatypeState::Unsubscribing;
    pushed.is_readonly = true;
    pushed.transactions = vec![transaction(
        &pushed.cuid,
        2,
        Operation::new_counter_increase(5),
    )];
    let pulled = server.process(&pushed, false).unwrap();
    assert_eq!(pulled.error, Some(PushPullError::ReadonlyViolation));
    assert!(!server.synced.checkpoints.contains_key(&pushed.cuid));
    assert_eq!(
        server.max_sub_gens.get(&pushed.cuid).copied(),
        Some(pushed.sub_gen)
    );
    assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 7);
    assert_eq!(server.synced.sseq_at_server, 1);
    assert_eq!(server.synced.history.len(), 1);
    let repeated = server.process(&pushed, false).unwrap();
    assert!(repeated.error.is_none());
    assert_eq!(repeated.state, DatatypeState::Disabled);
    assert!(server.synced.checkpoints.is_empty());
    assert_eq!(server.synced.sseq_at_server, 1);
    pushed.state = DatatypeState::Creating;
    assert!(matches!(
        server.process(&pushed, false).unwrap().error,
        Some(PushPullError::MissingSubscription(_))
    ));
    assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 7);
}

#[test]
#[instrument]
fn can_sync_bidirectionally_between_two_clients() {
    let f = TestFixture::new(get_test_ids!());
    let client1 = f.client("client1-1");
    let client2 = f.client("client1-2");
    let counter1 = f.create_counter(&client1);
    counter1.sync().unwrap();

    let counter2 = f.subscribe_counter(&client2);
    counter2.sync().unwrap();

    // === A: one-way convergence counter1 → counter2 ===
    for i in 0..5 {
        counter1.increase_by(i).unwrap();
    }
    // 0+1+2+3+4 = 10
    let expected_after_a = 10;
    assert_eq!(counter1.get_value(), expected_after_a);

    counter1.sync().unwrap();
    // counter1 must not re-apply its own transactions
    assert_eq!(counter1.get_value(), expected_after_a);

    counter2.sync().unwrap();
    assert_eq!(counter2.get_value(), expected_after_a);
    assert_eq!(counter1.get_value(), counter2.get_value());

    // === B: reverse convergence counter2 → counter1 ===
    counter2.increase_by(5).unwrap();
    counter2.increase_by(5).unwrap();
    let expected_after_b = expected_after_a + 10;

    counter2.sync().unwrap();
    assert_eq!(counter2.get_value(), expected_after_b);

    counter1.sync().unwrap();
    assert_eq!(counter1.get_value(), expected_after_b);

    // === C: bidirectional convergence after concurrent writes ===
    counter1.increase_by(3).unwrap();
    counter2.increase_by(7).unwrap();
    let expected_after_c = expected_after_b + 3 + 7;

    counter1.sync().unwrap();
    counter2.sync().unwrap();
    counter1.sync().unwrap(); // counter1 fetches counter2's tx
    assert_eq!(counter1.get_value(), expected_after_c);
    assert_eq!(counter2.get_value(), expected_after_c);

    // === D: checkpoint consistency and idempotent sync ===
    // both clients must see the same server state
    assert_eq!(counter1.get_server_version(), counter2.get_server_version());
    // all local transactions of each client must be confirmed by the server
    assert_eq!(
        counter1.get_client_version(),
        counter1.get_synced_client_version()
    );
    assert_eq!(
        counter2.get_client_version(),
        counter2.get_synced_client_version()
    );

    // repeated syncs must not change values (idempotent)
    let value_before = counter1.get_value();
    counter1.sync().unwrap();
    counter2.sync().unwrap();
    assert_eq!(counter1.get_value(), value_before);
    assert_eq!(counter2.get_value(), value_before);
}

#[test]
#[instrument]
fn can_unsubscribe_not_subscribed_client_gracefully() {
    let f = TestFixture::new(get_test_ids!());
    let client1 = f.client(get_test_func_name!());
    let counter1 = f.create_counter(&client1);

    counter1.sync().unwrap();
    assert_eq!(counter1.get_state(), DatatypeState::Subscribed);

    // Simulate the server losing the client's subscription.
    f.remove_client_subscription(&client1);

    // Unsubscribing a client whose server-side subscription is gone must complete
    // gracefully: no error, and the datatype reaches Disabled.
    // Manual mode: send_push_transaction_with_best_effort skips the event, so call
    // sync() explicitly to drive the Unsubscribing → Disabled transition.
    counter1.unsubscribe().unwrap();
    assert_eq!(counter1.get_state(), DatatypeState::Unsubscribing);
    counter1.sync().unwrap();
    assert_eq!(counter1.get_state(), DatatypeState::Disabled);
}
