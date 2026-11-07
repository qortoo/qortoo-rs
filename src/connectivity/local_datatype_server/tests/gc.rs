use super::*;

#[test]
fn can_hold_gc_acknowledgements_until_the_entire_push_buffer_is_drained() {
    let f = TestFixture::new(get_test_ids!());
    let client = f.client("writer");
    let writer = f.create_counter(&client);
    writer.increase_by(10).unwrap();
    writer.sync().unwrap();
    let peer_client = f.client("peer");
    let peer = f.subscribe_counter(&peer_client);
    peer.sync().unwrap();
    peer.increase_by(10).unwrap();
    peer.sync().unwrap();
    // Each transaction exceeds half the transport budget, so each request sends one.
    for _ in 0..3 {
        writer
            .transaction(
                "x".repeat(crate::defaults::DEFAULT_MAX_TRANSMISSION_SIZE as usize / 2 + 1),
                |counter| {
                    counter.increase()?;
                    Ok(())
                },
            )
            .unwrap();
    }
    let reports = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let observed = reports.clone();
    f.interceptor(&client).set_before_push(move |push| {
        observed.lock().push((
            push.sseq_gc_safe,
            push.checkpoint.sseq,
            push.transactions.len(),
        ));
    });
    writer.sync().unwrap();
    writer.sync().unwrap();
    writer.sync().unwrap();
    assert_eq!(*reports.lock(), vec![(1, 1, 1), (1, 3, 1), (1, 4, 1)]);
    assert_eq!(writer.get_value(), 23);
    assert_eq!(
        f.server().read().synced.checkpoints[&client.get_cuid()].sseq_gc_for_client,
        1
    );
    writer.sync().unwrap();
    assert_eq!(reports.lock()[3], (5, 5, 0));
    assert_eq!(
        f.server().read().synced.checkpoints[&client.get_cuid()].sseq_gc_for_client,
        5
    );
    peer.sync().unwrap();
    assert_eq!(f.server().read().synced.sseq_gc_safe, 2);
}

#[test]
fn can_preserve_the_bootstrap_floor_without_acknowledging_a_lost_snapshot() {
    let f = TestFixture::new(get_test_ids!());
    let creator = f.client("creator");
    let source = f.create_counter(&creator);
    source.increase_by(10).unwrap();
    source.sync().unwrap();
    source.sync().unwrap();
    assert_eq!(f.server().read().synced.sseq_gc_safe, 1);
    source.increase_by(2).unwrap();
    source.sync().unwrap();
    let client = f.client("reader");
    let reader = f.subscribe_counter(&client);
    let interceptor = f.interceptor(&client);
    interceptor.set_after_pull(|_| {
        Err(ConnectivityError::TimedOut("lost snapshot".into())
            .to_datatype_error()
            .mapping())
    });
    assert!(reader.sync().is_err());
    source.sync().unwrap();
    assert_eq!(f.server().read().synced.sseq_gc_safe, 1);
    assert_eq!(
        f.server().read().synced.checkpoints[&client.get_cuid()].sseq_gc_for_client,
        1
    );
    interceptor.set_before_push(|push| assert_eq!(push.sseq_gc_safe, 0));
    interceptor.set_after_pull(|_| Ok(()));
    reader.sync().unwrap();
    assert_eq!(reader.get_value(), 12);
    assert_eq!(f.server().read().synced.sseq_gc_safe, 1);
    interceptor.set_before_push(|push| assert_eq!(push.sseq_gc_safe, 2));
    reader.sync().unwrap();
    assert_eq!(f.server().read().synced.sseq_gc_safe, 2);
}

#[test]
fn can_include_readonly_gc_reports_and_ignore_delayed_confirmations() {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut writer = PushPullPack::new(&attr, DatatypeState::Creating);
    writer.transactions = (1..=3)
        .map(|seq| transaction(&writer.cuid, seq, Operation::new_counter_increase(1)))
        .collect();
    assert!(server.process(&writer, false).unwrap().error.is_none());
    let mut reader = PushPullPack::new(&attr, DatatypeState::Subscribing);
    reader.cuid = Cuid::new();
    reader.is_readonly = true;
    server.process(&reader, false).unwrap();
    writer.state = DatatypeState::Subscribed;
    writer.transactions.clear();
    writer.checkpoint = CheckPoint::new(3, 3);
    writer.sseq_gc_safe = 3;
    assert_eq!(server.process(&writer, false).unwrap().sseq_gc_safe, 0);
    reader.state = DatatypeState::Subscribed;
    reader.checkpoint.sseq = 3;
    reader.sseq_gc_safe = 2;
    assert_eq!(server.process(&reader, false).unwrap().sseq_gc_safe, 2);
    reader.sseq_gc_safe = 1;
    assert_eq!(server.process(&reader, false).unwrap().sseq_gc_safe, 2);
    reader.sseq_gc_safe = 2;
    assert_eq!(server.process(&reader, false).unwrap().sseq_gc_safe, 2);
    reader.state = DatatypeState::Unsubscribing;
    assert_eq!(server.process(&reader, false).unwrap().sseq_gc_safe, 3);
    writer.state = DatatypeState::Unsubscribing;
    assert_eq!(server.process(&writer, false).unwrap().sseq_gc_safe, 3);
    assert!(server.synced.checkpoints.is_empty());
}

#[test]
fn can_reset_gc_confirmation_before_automatic_resubscription() {
    let f = TestFixture::new(get_test_ids!());
    let client = f.client("client");
    let counter = f.create_counter(&client);
    counter.increase_by(10).unwrap();
    counter.sync().unwrap();
    counter.sync().unwrap();
    let interceptor = f.interceptor(&client);
    let reports = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let observed = reports.clone();
    let ids = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let observed_ids = ids.clone();
    interceptor.set_before_push(move |push| {
        observed.lock().push((push.state, push.sseq_gc_safe));
        observed_ids.lock().push(push.sub_gen);
    });
    counter.increase_by(5).unwrap();
    f.remove_client_subscription(&client);
    counter.sync().unwrap();
    assert_eq!(
        *reports.lock(),
        vec![
            (DatatypeState::Subscribed, 1),
            (DatatypeState::Subscribing, 0)
        ]
    );
    assert_eq!(counter.get_value(), 10);
    counter.sync().unwrap();
    assert_eq!(reports.lock()[2], (DatatypeState::Subscribed, 1));
    let ids = ids.lock();
    assert_ne!(ids[0], ids[1]);
    assert_eq!(ids[1], ids[2]);
}

#[rstest]
#[case::above_checkpoint(true)]
#[case::invalid_batch(false)]
fn can_reject_gc_reports_without_publishing_progress(#[case] above_checkpoint: bool) {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
    pushed.transactions = vec![transaction(
        &pushed.cuid,
        1,
        Operation::new_counter_increase(1),
    )];
    server.process(&pushed, false).unwrap();
    pushed.state = DatatypeState::Subscribed;
    pushed.checkpoint = CheckPoint::new(1, 1);
    pushed.sseq_gc_safe = if above_checkpoint { 2 } else { 1 };
    pushed.transactions = vec![transaction(
        &pushed.cuid,
        if above_checkpoint { 2 } else { 3 },
        Operation::new_counter_increase(10),
    )];
    let before = server.synced.checkpoints.clone();
    let pulled = server.process(&pushed, false).unwrap();
    assert!(matches!(
        pulled.error,
        Some(PushPullError::ProtocolViolation(_))
    ));
    assert_eq!(server.synced.checkpoints, before);
    assert_eq!(server.synced.sseq_at_server, 1);
    assert_eq!(server.synced.sseq_gc_safe, 0);
}
