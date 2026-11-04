use super::*;

fn sseqs<'a>(txs: impl IntoIterator<Item = &'a Arc<Transaction>>) -> Vec<u64> {
    txs.into_iter().map(|tx| tx.sseq).collect()
}

#[test]
fn can_retain_history_for_the_slowest_of_three_subscribers_until_unsubscribe() {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut writer = PushPullPack::new(&attr, DatatypeState::Creating);
    writer.transactions = (1..=100)
        .map(|cseq| transaction(&writer.cuid, cseq, Operation::new_counter_increase(1)))
        .collect();
    assert!(server.process(&writer, false).unwrap().error.is_none());

    let mut slow = PushPullPack::new(&attr, DatatypeState::Subscribing);
    slow.cuid = Cuid::new();
    slow.is_readonly = true;
    let mut reader = PushPullPack::new(&attr, DatatypeState::Subscribing);
    reader.cuid = Cuid::new();
    reader.is_readonly = true;
    for subscriber in [&slow, &reader] {
        assert!(server.process(subscriber, false).unwrap().error.is_none());
    }
    writer.state = DatatypeState::Subscribed;
    writer.transactions.clear();
    writer.checkpoint.sseq = 100;
    slow.state = DatatypeState::Subscribed;
    slow.checkpoint.sseq = 80;
    reader.state = DatatypeState::Subscribed;
    reader.checkpoint.sseq = 95;
    for subscriber in [&writer, &slow, &reader] {
        assert!(server.process(subscriber, false).unwrap().error.is_none());
    }
    assert_eq!(server.synced.sseq_history_prunable(), 80);
    assert_eq!(server.synced.sseq_history_pruned, 80);
    assert_eq!(
        sseqs(&server.synced.history),
        (81..=100).collect::<Vec<_>>()
    );

    // Other subscribers' successful syncs cannot release the idle reader's protection.
    assert!(server.process(&writer, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_pruned, 80);
    slow.state = DatatypeState::Unsubscribing;
    assert!(server.process(&slow, false).unwrap().error.is_none());
    assert!(!server.synced.checkpoints.contains_key(&slow.cuid));
    assert_eq!(server.synced.sseq_history_prunable(), 95);
    assert_eq!(server.synced.sseq_history_pruned, 95);
    assert_eq!(
        sseqs(&server.synced.history),
        (96..=100).collect::<Vec<_>>()
    );
}

#[test]
fn can_prune_acknowledged_history_without_invalidating_in_flight_pulls() {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut writer = PushPullPack::new(&attr, DatatypeState::Creating);
    writer.transactions = (1..=4)
        .map(|cseq| transaction(&writer.cuid, cseq, Operation::new_counter_increase(1)))
        .collect();
    assert!(server.process(&writer, false).unwrap().error.is_none());
    let mut reader = PushPullPack::new(&attr, DatatypeState::Subscribing);
    reader.cuid = crate::types::uid::Cuid::new();
    reader.is_readonly = true;
    assert!(server.process(&reader, false).unwrap().error.is_none());
    writer.state = DatatypeState::Subscribed;
    writer.transactions.clear();
    writer.checkpoint.sseq = 4;
    assert!(server.process(&writer, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_pruned, 0);
    assert_eq!(server.synced.history.len(), 4);

    reader.state = DatatypeState::Subscribed;
    reader.checkpoint.sseq = 2;
    let in_flight = server.process(&reader, false).unwrap();
    assert!(in_flight.error.is_none());
    assert_eq!(server.synced.sseq_history_prunable(), 2);
    assert_eq!(server.synced.sseq_history_pruned, 2);
    assert_eq!(sseqs(&server.synced.history), vec![3, 4]);
    assert_eq!(sseqs(&in_flight.transactions), vec![3, 4]);
    assert!(Arc::ptr_eq(
        &in_flight.transactions[0],
        &server.synced.history[0]
    ));
    let retained = Arc::downgrade(&in_flight.transactions[0]);
    reader.checkpoint.sseq = 4;
    assert!(server.process(&reader, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_pruned, 4);
    assert!(server.synced.history.is_empty());
    assert_eq!(server.synced.sseq_at_server, 4);
    assert_eq!(server.synced.lamport, 4);
    assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 4);
    assert_eq!(
        server.synced.checkpoints[&writer.cuid].checkpoint(),
        CheckPoint::new(4, 4)
    );
    assert_eq!(in_flight.transactions[0].operations.len(), 1);
    assert!(retained.upgrade().is_some());
    drop(in_flight);
    assert!(retained.upgrade().is_none());
}

#[rstest]
#[case::creation_retry(DatatypeState::Creating)]
#[case::subscribe_or_create_retry(DatatypeState::SubscribingOrCreating)]
#[case::subscribed(DatatypeState::Subscribed)]
#[case::unsubscribing(DatatypeState::Unsubscribing)]
fn can_reject_pruned_progress_before_accepting_new_transactions(#[case] state: DatatypeState) {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
    let accepted = transaction(&pushed.cuid, 1, Operation::new_counter_increase(7));
    pushed.transactions = vec![accepted.clone()];
    assert!(server.process(&pushed, false).unwrap().error.is_none());
    pushed.state = DatatypeState::Subscribed;
    pushed.transactions.clear();
    pushed.checkpoint.sseq = 1;
    assert!(server.process(&pushed, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_pruned, 1);
    let checkpoints = server.synced.checkpoints.clone();
    pushed.state = state;
    pushed.checkpoint.sseq = 0;
    pushed.transactions = vec![
        accepted,
        transaction(&pushed.cuid, 2, Operation::new_counter_increase(10)),
    ];
    let pulled = server.process(&pushed, false).unwrap();
    let Some(PushPullError::ProtocolViolation(message)) = pulled.error else {
        panic!("expected pruned history protocol error");
    };
    assert_eq!(message, "requested server history has been pruned");
    assert_eq!(pulled.state, DatatypeState::Disabled);
    assert!(pulled.transactions.is_empty());
    assert_eq!(server.synced.sseq_at_server, 1);
    assert_eq!(server.synced.lamport, 1);
    assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 7);
    assert!(server.synced.history.is_empty());
    assert_eq!(server.synced.sseq_history_pruned, 1);
    if state == DatatypeState::Unsubscribing {
        assert!(server.synced.checkpoints.is_empty());
    } else {
        assert_eq!(server.synced.checkpoints, checkpoints);
    }
}

#[test]
fn can_deduplicate_retransmissions_at_the_pruned_boundary() {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
    let accepted = transaction(&pushed.cuid, 1, Operation::new_counter_increase(7));
    pushed.transactions = vec![accepted.clone()];
    assert!(server.process(&pushed, false).unwrap().error.is_none());
    pushed.state = DatatypeState::Subscribed;
    pushed.checkpoint.sseq = 1;
    pushed.transactions.clear();
    assert!(server.process(&pushed, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_pruned, 1);
    pushed.transactions = vec![
        accepted,
        transaction(&pushed.cuid, 2, Operation::new_counter_increase(3)),
    ];
    let accepted = server.process(&pushed, false).unwrap();
    assert!(accepted.error.is_none());
    assert_eq!(accepted.checkpoint, CheckPoint::new(2, 2));
    let repeated = server.process(&pushed, false).unwrap();
    assert!(repeated.error.is_none());
    assert_eq!(repeated.checkpoint, accepted.checkpoint);
    assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 10);
    assert_eq!(server.synced.sseq_at_server, 2);
    assert_eq!(server.synced.history.len(), 1);
    assert_eq!(server.synced.history[0].sseq, 2);
    assert_eq!(server.synced.sseq_history_pruned, 1);
    assert_eq!(
        server.synced.checkpoints[&pushed.cuid].checkpoint(),
        CheckPoint::new(1, 2)
    );
}

#[test]
fn can_defer_pruning_after_failed_unsubscribe_and_preserve_a_delayed_pull() {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut writer = PushPullPack::new(&attr, DatatypeState::Creating);
    writer.transactions = (1..=2)
        .map(|cseq| transaction(&writer.cuid, cseq, Operation::new_counter_increase(1)))
        .collect();
    assert!(server.process(&writer, false).unwrap().error.is_none());
    let mut reader = PushPullPack::new(&attr, DatatypeState::Subscribing);
    reader.cuid = crate::types::uid::Cuid::new();
    assert!(server.process(&reader, false).unwrap().error.is_none());
    reader.state = DatatypeState::Subscribed;
    reader.checkpoint.sseq = 2;
    assert!(server.process(&reader, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_pruned, 0);
    writer.state = DatatypeState::Unsubscribing;
    writer.is_readonly = true;
    writer.transactions = vec![transaction(
        &writer.cuid,
        3,
        Operation::new_counter_increase(10),
    )];
    let failed = server.process(&writer, false).unwrap();
    assert_eq!(failed.error, Some(PushPullError::ReadonlyViolation));
    assert!(!server.synced.checkpoints.contains_key(&writer.cuid));
    assert_eq!(server.synced.sseq_history_prunable(), 2);
    assert_eq!(server.synced.sseq_history_pruned, 0);
    assert_eq!(server.synced.history.len(), 2);
    assert_eq!(server.synced.sseq_at_server, 2);
    assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 2);

    // The stored acknowledgement is newer, but this request still needs the retained history.
    reader.checkpoint.sseq = 0;
    let delayed = server.process(&reader, false).unwrap();
    assert!(delayed.error.is_none());
    assert_eq!(sseqs(&delayed.transactions), vec![1, 2]);
    assert_eq!(server.synced.sseq_history_pruned, 2);
    assert!(server.synced.history.is_empty());
    assert_eq!(delayed.transactions[0].operations.len(), 1);
}

#[test]
fn can_prune_to_the_head_after_the_last_unsubscribe() {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
    pushed.transactions = vec![transaction(
        &pushed.cuid,
        1,
        Operation::new_counter_increase(7),
    )];
    assert!(server.process(&pushed, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_pruned, 0);
    pushed.state = DatatypeState::Unsubscribing;
    pushed.transactions.clear();
    let pulled = server.process(&pushed, false).unwrap();
    assert!(pulled.error.is_none());
    assert_eq!(server.synced.sseq_history_prunable(), 1);
    assert_eq!(server.synced.sseq_history_pruned, 1);
    assert!(server.synced.checkpoints.is_empty());
    assert!(server.synced.history.is_empty());
    assert_eq!(server.synced.sseq_at_server, 1);
    assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 7);
}

#[test]
fn can_disable_a_client_and_discard_pending_transactions_when_history_is_pruned() {
    let f = TestFixture::new(get_test_ids!());
    let client = f.client("client");
    let counter = f.create_counter(&client);
    counter.increase_by(7).unwrap();
    counter.sync().unwrap();
    counter.sync().unwrap();
    let server = f.server();
    let wired = server
        .read()
        .get_wired_datatype(&client.get_cuid())
        .unwrap();
    assert_eq!(server.read().synced.sseq_history_pruned, 1);
    counter.increase_by(5).unwrap();
    assert_eq!(wired.core.mutable.read().push_buffer.iter().count(), 1);
    let interceptor = f.interceptor(&client);
    interceptor.set_before_push(|push| {
        push.checkpoint.sseq = 0;
        push.sseq_gc_safe = 0;
    });
    let error = counter.sync().unwrap_err();
    let DatatypeError::ServerRejected(crate::ServerRejectReason::ProtocolViolation(message)) =
        error
    else {
        panic!("expected pruned history rejection");
    };
    assert_eq!(message, "requested server history has been pruned");
    assert_eq!(counter.get_state(), DatatypeState::Disabled);
    assert!(
        wired
            .core
            .mutable
            .read()
            .push_buffer
            .iter()
            .next()
            .is_none()
    );
    let backend = server.read();
    assert_eq!(backend.synced.crdt.as_counter().unwrap().value(), 7);
    assert_eq!(backend.synced.sseq_at_server, 1);
    assert_eq!(backend.synced.sseq_history_pruned, 1);
    assert!(backend.synced.history.is_empty());
    assert_eq!(
        backend.synced.checkpoints[&client.get_cuid()].checkpoint(),
        CheckPoint::new(1, 1)
    );
}

#[test]
fn can_track_readonly_acknowledgements_without_advancing_on_snapshot_delivery() {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut writer = PushPullPack::new(&attr, DatatypeState::Creating);
    writer.transactions = vec![transaction(
        &writer.cuid,
        1,
        Operation::new_counter_increase(7),
    )];
    let created = server.process(&writer, false).unwrap();
    assert_eq!(server.synced.sseq_history_prunable(), 0);
    assert_eq!(created.checkpoint.sseq, 1);
    writer.state = DatatypeState::Subscribed;
    writer.transactions.clear();
    writer.checkpoint.sseq = 1;
    assert!(server.process(&writer, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_prunable(), 1);

    let mut reader = PushPullPack::new(&attr, DatatypeState::Subscribing);
    reader.cuid = crate::types::uid::Cuid::new();
    reader.is_readonly = true;
    let snapshot = server.process(&reader, false).unwrap();
    assert!(snapshot.snapshot_transaction.is_some());
    assert_eq!(server.synced.sseq_history_prunable(), 1);
    assert_eq!(server.synced.checkpoints[&reader.cuid].sseq_at_client, 1);
    writer.transactions = vec![transaction(
        &writer.cuid,
        2,
        Operation::new_counter_increase(3),
    )];
    assert!(server.process(&writer, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_prunable(), 1);
    writer.transactions.clear();
    writer.checkpoint.sseq = 2;
    assert!(server.process(&writer, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_prunable(), 1);

    // A repeated subscribe may provide a newer snapshot, but is not its acknowledgement.
    let repeated = server.process(&reader, false).unwrap();
    assert_eq!(repeated.checkpoint.sseq, 2);
    assert_eq!(server.synced.sseq_history_prunable(), 1);
    reader.state = DatatypeState::Subscribed;
    reader.checkpoint.sseq = 1;
    let replay = server.process(&reader, false).unwrap();
    assert_eq!(replay.transactions.len(), 1);
    assert_eq!(server.synced.sseq_history_prunable(), 1);
    reader.checkpoint.sseq = 2;
    assert!(server.process(&reader, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_prunable(), 2);
    reader.checkpoint.sseq = 1;
    let delayed = server.process(&reader, false).unwrap();
    assert!(matches!(
        delayed.error,
        Some(PushPullError::ProtocolViolation(_))
    ));
    assert!(delayed.transactions.is_empty());
    assert_eq!(server.synced.sseq_history_pruned, 2);
    assert_eq!(server.synced.sseq_history_prunable(), 2);
    assert_eq!(server.synced.checkpoints[&reader.cuid].sseq_at_client, 2);

    reader.state = DatatypeState::Unsubscribing;
    reader.checkpoint.sseq = 2;
    assert!(server.process(&reader, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_prunable(), 2);
    assert!(!server.synced.checkpoints.contains_key(&reader.cuid));
    writer.state = DatatypeState::Unsubscribing;
    assert!(server.process(&writer, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq_history_prunable(), 2);
    assert!(server.synced.checkpoints.is_empty());
}

#[derive(Clone, Copy, Debug)]
enum InvalidPushKind {
    FutureSequence,
    WrongDatatype,
    MissingSubscription,
    FailedBatch,
}

#[rstest]
#[case::future_sequence(InvalidPushKind::FutureSequence)]
#[case::wrong_datatype(InvalidPushKind::WrongDatatype)]
#[case::missing_subscription(InvalidPushKind::MissingSubscription)]
#[case::failed_batch(InvalidPushKind::FailedBatch)]
fn can_reject_invalid_pushes_without_advancing_applied_progress(#[case] kind: InvalidPushKind) {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut writer = PushPullPack::new(&attr, DatatypeState::Creating);
    writer.transactions = vec![transaction(
        &writer.cuid,
        1,
        Operation::new_counter_increase(7),
    )];
    assert!(server.process(&writer, false).unwrap().error.is_none());
    let mut peer = PushPullPack::new(&attr, DatatypeState::Subscribing);
    peer.cuid = crate::types::uid::Cuid::new();
    server.process(&peer, false).unwrap();
    peer.state = DatatypeState::Subscribed;
    peer.transactions = vec![transaction(
        &peer.cuid,
        1,
        Operation::new_counter_increase(3),
    )];
    server.process(&peer, false).unwrap();
    assert_eq!(server.synced.sseq_at_server, 2);

    writer.state = DatatypeState::Subscribed;
    writer.checkpoint.sseq = 1;
    writer.transactions = vec![transaction(
        &writer.cuid,
        2,
        Operation::new_counter_increase(5),
    )];
    match kind {
        // This request must be checked against the head before accepting its new transaction.
        InvalidPushKind::FutureSequence => writer.checkpoint.sseq = 3,
        InvalidPushKind::WrongDatatype => writer.duid = Duid::new(),
        InvalidPushKind::MissingSubscription => writer.cuid = crate::types::uid::Cuid::new(),
        InvalidPushKind::FailedBatch => {
            writer.transactions = vec![transaction(
                &writer.cuid,
                3,
                Operation::new_counter_increase(5),
            )]
        }
    }
    let rejected = server.process(&writer, false).unwrap();
    assert!(rejected.error.is_some());
    assert_eq!(server.synced.sseq_history_prunable(), 0);
    assert_eq!(server.synced.sseq_at_server, 2);
    assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 10);
    assert_eq!(
        server.synced.checkpoints[&attr.get_cuid()].sseq_at_client,
        0
    );
    assert_eq!(
        server.synced.checkpoints[&attr.get_cuid()].cseq_at_server,
        1
    );
}

#[rstest]
#[case::creation(DatatypeState::Creating, false)]
#[case::subscribe_or_create(DatatypeState::SubscribingOrCreating, false)]
#[case::creation_retry(DatatypeState::Creating, true)]
#[case::subscribe_or_create_retry(DatatypeState::SubscribingOrCreating, true)]
#[case::subscribed(DatatypeState::Subscribed, true)]
#[case::subscription_retry(DatatypeState::Subscribing, true)]
#[case::unsubscribing(DatatypeState::Unsubscribing, true)]
fn can_report_a_consistent_error_for_a_checkpoint_beyond_the_server_head(
    #[case] state: DatatypeState,
    #[case] created: bool,
) {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
    if created {
        assert!(server.process(&pushed, false).unwrap().error.is_none());
    }
    pushed.state = state;
    pushed.checkpoint.sseq = 1;
    let pulled = server.process(&pushed, false).unwrap();
    assert_eq!(
        pulled.error,
        Some(PushPullError::ProtocolViolation(
            "checkpoint exceeds the server head".into()
        ))
    );
    assert_eq!(pulled.state, DatatypeState::Disabled);
    assert_eq!(server.synced.sseq_at_server, 0);
}

#[test]
fn can_acknowledge_an_existing_boundary_without_a_prior_response() {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut writer = PushPullPack::new(&attr, DatatypeState::Creating);
    writer.transactions = vec![transaction(
        &writer.cuid,
        1,
        Operation::new_counter_increase(7),
    )];
    server.process(&writer, false).unwrap();
    let mut peer = PushPullPack::new(&attr, DatatypeState::Subscribing);
    peer.cuid = crate::types::uid::Cuid::new();
    server.process(&peer, false).unwrap();
    peer.state = DatatypeState::Subscribed;
    peer.transactions = vec![transaction(
        &peer.cuid,
        1,
        Operation::new_counter_increase(3),
    )];
    server.process(&peer, false).unwrap();

    // The writer's previous response only covered sequence 1; it reports applying 2 elsewhere.
    writer.state = DatatypeState::Subscribed;
    writer.transactions.clear();
    writer.checkpoint.sseq = 2;
    let pulled = server.process(&writer, false).unwrap();
    assert!(pulled.error.is_none());
    assert_eq!(server.synced.checkpoints[&writer.cuid].sseq_at_client, 2);
    assert!(pulled.transactions.is_empty());
}

#[test]
fn can_exclude_registration_only_handles_from_applied_progress() {
    let f = TestFixture::new(get_test_ids!());
    let creator = f.client("creator");
    let pending = f.client("pending");
    let source = f.create_counter(&creator);
    source.increase().unwrap();
    source.sync().unwrap();
    source.sync().unwrap();
    let waiting = f.subscribe_counter(&pending);
    let server = f.server();
    assert_eq!(server.read().synced.checkpoints.len(), 1);
    assert_eq!(server.read().synced.sseq_history_prunable(), 1);
    let unsubscribe = PushPullPack::new(&waiting.get_attr(), DatatypeState::Unsubscribing);
    assert!(
        server
            .write()
            .process(&unsubscribe, false)
            .unwrap()
            .error
            .is_none()
    );
    assert!(!server.read().wired_map.contains_key(&pending.get_cuid()));
    source.unsubscribe().unwrap();
    source.sync().unwrap();
    assert!(
        f.connectivity
            .get_local_datatype_server(&f.resource_id)
            .is_none()
    );
}
