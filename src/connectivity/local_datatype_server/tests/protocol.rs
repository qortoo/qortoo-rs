use super::*;

fn push_no_change(_push: &mut PushPullPack) {}

#[rstest]
#[case::uncreated(false)]
#[case::created(true)]
fn can_reject_unsupported_deletion_without_changing_backend_state(#[case] created: bool) {
    use crate::datatypes::{wired::WiredDatatype, wired_interceptor::WiredInterceptor};

    let attr = new_attribute!(DataType::Counter);
    let wired = WiredDatatype::new_arc_for_test(
        attr.clone(),
        DatatypeState::Creating,
        WiredInterceptor::new_arc(),
    );
    let mut server = LocalDatatypeServer::new(&attr);
    let (sender, receiver) = crossbeam_channel::unbounded();
    server.insert_client_item(wired.clone(), sender);
    let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
    if created {
        pushed.transactions = vec![transaction(
            &pushed.cuid,
            1,
            Operation::new_counter_increase(7),
        )];
        assert!(server.process(&pushed, false).unwrap().error.is_none());
        pushed.state = DatatypeState::Subscribed;
        pushed.checkpoint.sseq = 1;
        pushed.transactions.clear();
        assert!(server.process(&pushed, false).unwrap().error.is_none());
    }
    let duid = server.duid.clone();
    let crdt = server.synced.crdt.serialize();
    let lamport = server.synced.lamport;
    let sseq = server.synced.sseq;
    let history = server.synced.history.clone();
    let checkpoints = server.synced.checkpoints.clone();
    let safe_sseq = server.synced.safe_sseq();
    let pruned_sseq = server.synced.pruned_sseq;

    pushed.state = DatatypeState::Deleting;
    pushed.safe_sseq = u64::MAX;
    pushed.transactions = vec![transaction(
        &pushed.cuid,
        if created { 2 } else { 1 },
        Operation::new_counter_increase(10),
    )];
    let pulled = server.process(&pushed, true).unwrap();
    let Some(PushPullError::ProtocolViolation(message)) = pulled.error else {
        panic!("expected unsupported deletion protocol error");
    };
    assert_eq!(message, "datatype deletion is not supported");
    assert_eq!(pulled.state, DatatypeState::Disabled);
    assert!(pulled.transactions.is_empty());
    assert!(pulled.snapshot_transaction.is_none());
    assert_eq!(pulled.safe_sseq, safe_sseq);
    assert_eq!(server.duid, duid);
    assert_eq!(server.is_created(), created);
    assert_eq!(server.synced.crdt.serialize(), crdt);
    assert_eq!(server.synced.lamport, lamport);
    assert_eq!(server.synced.sseq, sseq);
    assert_eq!(server.synced.history, history);
    assert_eq!(server.synced.pruned_sseq, pruned_sseq);
    assert_eq!(server.synced.checkpoints, checkpoints);
    assert_eq!(server.wired_map.len(), 1);
    assert!(Arc::ptr_eq(
        &server.get_wired_datatype(&pushed.cuid).unwrap(),
        &wired
    ));
    assert_eq!(server.sender_map.len(), 1);
    assert!(server.sender_map.contains_key(&pushed.cuid));
    assert!(receiver.is_empty());
}

#[rstest]
#[case::duplicate(&[2, 3, 3, 4])]
#[case::backward_sequence(&[2, 3, 2, 4])]
fn can_reject_nonsequential_new_transactions_without_publishing_batch(#[case] sequences: &[u64]) {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
    let accepted = transaction(&pushed.cuid, 1, Operation::new_counter_increase(7));
    pushed.transactions = vec![accepted.clone()];
    assert!(server.process(&pushed, false).unwrap().error.is_none());
    let before = server.synced.crdt.serialize();
    let history = server.synced.history.clone();
    let checkpoints = server.synced.checkpoints.clone();

    pushed.state = DatatypeState::Subscribed;
    pushed.checkpoint.sseq = 1;
    pushed.transactions = sequences
        .iter()
        .enumerate()
        .map(|(index, &cseq)| {
            transaction(
                &pushed.cuid,
                cseq,
                Operation::new_counter_increase(index as i64 + 10),
            )
        })
        .collect();
    let failed = server.process(&pushed, false).unwrap();
    assert!(matches!(
        failed.error,
        Some(PushPullError::ProtocolViolation(_))
    ));
    assert_eq!(server.synced.crdt.serialize(), before);
    assert_eq!(server.synced.history, history);
    assert_eq!(server.synced.pruned_sseq, 0);
    assert_eq!(server.synced.checkpoints, checkpoints);
    assert_eq!(server.synced.sseq, 1);
    assert_eq!(server.synced.lamport, 1);

    // A retry may still include transactions accepted by an earlier request.
    pushed.transactions = vec![
        accepted,
        transaction(&pushed.cuid, 2, Operation::new_counter_increase(10)),
        transaction(&pushed.cuid, 3, Operation::new_counter_increase(20)),
    ];
    assert!(server.process(&pushed, false).unwrap().error.is_none());
    assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 37);
    assert_eq!(server.synced.history.len(), 2);
    assert_eq!(server.synced.pruned_sseq, 1);
    assert_eq!(server.synced.sseq, 3);
    assert_eq!(
        server.synced.checkpoints[&pushed.cuid],
        CheckPoint::new(1, 3)
    );
}

#[rstest]
#[case(DataType::Counter)]
#[case(DataType::Variable)]
fn can_reject_a_late_batch_failure_without_advancing_backend_state(#[case] data_type: DataType) {
    let attr = new_attribute!(data_type);
    let mut server = LocalDatatypeServer::new(&attr);
    let operation = |value: i64| match data_type {
        DataType::Counter => Operation::new_counter_increase(value),
        DataType::Variable => {
            Operation::new_variable_set(value.to_string().into_bytes().into_boxed_slice())
        }
        _ => unreachable!(),
    };
    let mut pushed = PushPullPack::new(&attr, DatatypeState::Creating);
    pushed.transactions = vec![transaction(&pushed.cuid, 1, operation(7))];
    assert!(server.process(&pushed, false).unwrap().error.is_none());
    let before = server.synced.crdt.serialize();
    let history = server.synced.history.clone();
    let checkpoints = server.synced.checkpoints.clone();
    pushed.state = DatatypeState::Subscribed;
    pushed.transactions = vec![
        transaction(&pushed.cuid, 2, operation(10)),
        transaction(&pushed.cuid, 3, operation(20)),
    ];
    let mut invalid = Operation::new_snapshot(Vec::new().into_boxed_slice());
    invalid.set_lamport(50);
    Arc::make_mut(&mut pushed.transactions[1]).push_operation(invalid);
    let failed = server.process(&pushed, false).unwrap();
    assert!(matches!(
        failed.error,
        Some(PushPullError::ProtocolViolation(_))
    ));
    assert_eq!(server.synced.crdt.serialize(), before);
    assert_eq!(server.synced.history, history);
    assert_eq!(server.synced.pruned_sseq, 0);
    assert_eq!(server.synced.checkpoints, checkpoints);
    assert_eq!(server.synced.sseq, 1);
    assert_eq!(server.synced.lamport, 1);

    Arc::make_mut(&mut pushed.transactions[1]).operations.pop();
    assert!(server.process(&pushed, false).unwrap().error.is_none());
    assert_eq!(server.synced.sseq, 3);
    assert_eq!(server.synced.lamport, 3);
    let after = server.synced.crdt.serialize();
    assert!(server.process(&pushed, false).unwrap().error.is_none());
    pushed.transactions.clear();
    assert!(server.process(&pushed, false).unwrap().error.is_none());
    assert_eq!(server.synced.crdt.serialize(), after);
    assert_eq!(server.synced.history.len(), 3);
    assert_eq!(server.synced.sseq, 3);
}

#[derive(Clone, Copy, Debug)]
enum CreationMutation {
    WrongOperation,
    WrongOrigin,
    SequenceGap,
    ZeroLamport,
    NilDuid,
}

#[rstest]
#[case::wrong_operation(CreationMutation::WrongOperation, DatatypeState::Creating)]
#[case::wrong_origin(CreationMutation::WrongOrigin, DatatypeState::Creating)]
#[case::sequence_gap(CreationMutation::SequenceGap, DatatypeState::Creating)]
#[case::zero_lamport(CreationMutation::ZeroLamport, DatatypeState::Creating)]
#[case::nil_duid(CreationMutation::NilDuid, DatatypeState::Creating)]
#[case::nil_duid_subscribe_or_create(
    CreationMutation::NilDuid,
    DatatypeState::SubscribingOrCreating
)]
fn can_keep_a_failed_creation_unpublished(
    #[case] mutation: CreationMutation,
    #[case] state: DatatypeState,
) {
    let attr = new_attribute!(DataType::Counter);
    let mut server = LocalDatatypeServer::new(&attr);
    let mut pushed = PushPullPack::new(&attr, state);
    let mut tx = (*transaction(&pushed.cuid, 1, Operation::new_counter_increase(7))).clone();
    match mutation {
        CreationMutation::WrongOperation => {
            let mut invalid = Operation::new_variable_set(b"7".to_vec().into_boxed_slice());
            invalid.set_lamport(2);
            tx.push_operation(invalid);
        }
        CreationMutation::WrongOrigin => tx.cuid = crate::types::uid::Cuid::new(),
        CreationMutation::SequenceGap => tx.cseq = 2,
        CreationMutation::ZeroLamport => tx.operations[0].set_lamport(0),
        CreationMutation::NilDuid => pushed.duid = Duid::new_nil(),
    }
    pushed.transactions.push(Arc::new(tx));
    let duid = server.duid.clone();
    assert!(server.process(&pushed, false).unwrap().error.is_some());
    assert!(!server.is_created());
    assert_eq!(server.duid, duid);
    assert!(server.synced.history.is_empty());
    assert!(server.synced.checkpoints.is_empty());
    assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 0);
    assert_eq!(server.synced.lamport, 0);
    assert_eq!(server.synced.sseq, 0);
    pushed.duid = attr.get_duid();
    pushed.transactions = vec![transaction(
        &pushed.cuid,
        1,
        Operation::new_counter_increase(7),
    )];
    assert!(server.process(&pushed, false).unwrap().error.is_none());
    assert_eq!(server.synced.crdt.as_counter().unwrap().value(), 7);
    assert!(server.is_created());
}

#[rstest]
#[case::create(false)]
#[case::subscribe_or_create(true)]
fn can_subscribe_before_the_creator_applies_its_accepted_response(
    #[case] create_or_subscribe: bool,
) {
    let f = TestFixture::new(get_test_ids!());
    let creator = f.client("creator");
    let subscriber = f.client("subscriber");
    let builder = if create_or_subscribe {
        creator.subscribe_or_create_datatype(f.key.clone())
    } else {
        creator.create_datatype(f.key.clone())
    };
    let source = builder.build_counter().unwrap();
    source.increase_by(7).unwrap();
    let interceptor = f.interceptor(&creator);
    interceptor
        .set_after_pull(|_| Err(DatatypeError::SyncFailed("response lost".into()).mapping()));
    assert!(source.sync().is_err());
    assert_eq!(source.get_server_version(), 0);
    let replica = f.subscribe_counter(&subscriber);
    replica.sync().unwrap();
    assert_eq!(replica.get_value(), 7);
    assert_eq!(replica.get_server_version(), 1);
    replica.increase_by(3).unwrap();
    replica.sync().unwrap();
    interceptor.set_after_pull(|_| Ok(()));
    source.sync().unwrap();
    replica.sync().unwrap();
    assert_eq!(source.get_value(), 10);
    assert_eq!(replica.get_value(), 10);
    assert_eq!(source.get_server_version(), 2);
}

fn push_set_readonly(push: &mut PushPullPack) {
    push.is_readonly = true;
}
fn push_set_new_duid(push: &mut PushPullPack) {
    push.duid = Duid::new();
}
fn push_set_variable_type(push: &mut PushPullPack) {
    push.r#type = DataType::Variable;
}
fn push_set_disabled_state(push: &mut PushPullPack) {
    push.state = DatatypeState::Disabled;
}
fn push_add_transaction(push: &mut PushPullPack) {
    push.transactions.push(Arc::new(Transaction::default()));
}

fn assert_push_pull_pack(
    pulled: &PushPullPack,
    is_readonly: bool,
    cp: CheckPoint,
    error: Option<PushPullError>,
    tx_len: usize,
) {
    assert_eq!(pulled.is_readonly, is_readonly);
    assert_eq!(pulled.checkpoint, cp);
    if error.is_some() {
        assert_eq!(pulled.state, DatatypeState::Disabled);
    } else {
        assert_eq!(pulled.state, DatatypeState::Subscribed);
    }

    assert_eq!(pulled.error, error);
    assert_eq!(pulled.transactions.len(), tx_len);
}

#[derive(Clone, Copy, Debug)]
enum CreatingScenario {
    Readonly,
    Normal,
    DuplicateRetry,
    AlreadyCreatedDifferentDuid,
}

struct CreatingExpectation {
    pre_create: bool,
    modify_push: fn(&mut PushPullPack),
    is_readonly: bool,
    error: Option<PushPullError>,
    checkpoint: CheckPoint,
    is_created: bool,
    history_len: usize,
}

impl CreatingScenario {
    fn expectation(self) -> CreatingExpectation {
        match self {
            Self::Readonly => CreatingExpectation {
                pre_create: false,
                modify_push: push_set_readonly,
                is_readonly: true,
                error: Some(PushPullError::ReadonlyViolation),
                checkpoint: CheckPoint::new(0, 0),
                is_created: false,
                history_len: 0,
            },
            Self::Normal => CreatingExpectation {
                pre_create: false,
                modify_push: push_no_change,
                is_readonly: false,
                error: None,
                checkpoint: CheckPoint::new(10, 10),
                is_created: true,
                history_len: 10,
            },
            Self::DuplicateRetry => CreatingExpectation {
                pre_create: true,
                modify_push: push_no_change,
                is_readonly: false,
                error: None,
                checkpoint: CheckPoint::new(10, 10),
                is_created: true,
                history_len: 10,
            },
            Self::AlreadyCreatedDifferentDuid => CreatingExpectation {
                pre_create: true,
                modify_push: push_set_new_duid,
                is_readonly: false,
                error: Some(PushPullError::CreateFailed("already exist".to_string())),
                checkpoint: CheckPoint::new(0, 0),
                is_created: true,
                history_len: 10,
            },
        }
    }
}

#[rstest]
#[case::readonly(CreatingScenario::Readonly)]
#[case::normal(CreatingScenario::Normal)]
#[case::duplicate(CreatingScenario::DuplicateRetry)]
#[case::already_created(CreatingScenario::AlreadyCreatedDifferentDuid)]
#[instrument]
fn can_process_creating(#[case] scenario: CreatingScenario) {
    let exp = scenario.expectation();
    let f = TestFixture::new(get_test_ids!());
    let client1 = f.client(get_test_func_name!());
    let counter1 = f.create_counter(&client1);
    for i in 0..10 {
        counter1.increase_by(i).unwrap();
    }

    let server = f.server();
    let wired_interceptor1 = f.interceptor(&client1);

    if exp.pre_create {
        wired_interceptor1.set_after_pull(|_pull| {
            Err(ConnectivityError::TimedOut("".to_owned())
                .to_datatype_error()
                .mapping())
        });
        let _ = counter1.sync();
        assert!(server.read().is_created());
    }

    let expected_error_for_pull = exp.error.clone();
    let modify_push = exp.modify_push;

    wired_interceptor1
        .set_before_push(move |push| {
            modify_push(push);
        })
        .set_after_pull(move |pull| {
            assert_push_pull_pack(
                pull,
                exp.is_readonly,
                exp.checkpoint,
                expected_error_for_pull.clone(),
                0,
            );
            Ok(())
        });

    let sync_result = counter1.sync();
    if let Some(ref sppe) = exp.error {
        let err = sync_result.unwrap_err();
        if matches!(sppe, PushPullError::ReadonlyViolation) {
            assert!(matches!(err, DatatypeError::ReadonlyViolation));
        } else {
            assert!(matches!(err, DatatypeError::ServerRejected(_)));
        }
        assert_eq!(counter1.get_state(), DatatypeState::Disabled);
    } else {
        assert!(sync_result.is_ok());
        assert_eq!(counter1.get_state(), DatatypeState::Subscribed);
    }
    assert_eq!(server.read().is_created(), exp.is_created);
    assert_eq!(server.read().synced.history.len(), exp.history_len);
}

#[derive(Clone, Copy, Debug)]
enum SubscribingScenario {
    Success,
    NotCreated,
    TypeMismatch,
    WithTransactions,
}

struct SubscribingExpectation {
    creator_sync: bool,
    modify_push: fn(&mut PushPullPack),
    error: Option<PushPullError>,
    checkpoint: CheckPoint,
}

impl SubscribingScenario {
    fn expectation(self) -> SubscribingExpectation {
        match self {
            Self::Success => SubscribingExpectation {
                creator_sync: true,
                modify_push: push_no_change,
                error: None,
                checkpoint: CheckPoint::new(5, 0),
            },
            Self::NotCreated => SubscribingExpectation {
                creator_sync: false,
                modify_push: push_no_change,
                error: Some(PushPullError::ResourceNotFound("".to_string())),
                checkpoint: CheckPoint::new(0, 0),
            },
            Self::TypeMismatch => SubscribingExpectation {
                creator_sync: true,
                modify_push: push_set_variable_type,
                error: Some(PushPullError::ResourceNotFound("".to_string())),
                checkpoint: CheckPoint::new(0, 0),
            },
            Self::WithTransactions => SubscribingExpectation {
                creator_sync: true,
                modify_push: push_add_transaction,
                error: Some(PushPullError::ProtocolViolation("".to_string())),
                checkpoint: CheckPoint::new(0, 0),
            },
        }
    }
}

#[rstest]
#[case::success(SubscribingScenario::Success)]
#[case::not_created(SubscribingScenario::NotCreated)]
#[case::type_mismatch(SubscribingScenario::TypeMismatch)]
#[case::with_transactions(SubscribingScenario::WithTransactions)]
#[instrument]
fn can_process_subscribing(#[case] scenario: SubscribingScenario) {
    let exp = scenario.expectation();
    let f = TestFixture::new(get_test_ids!());

    let client1 = f.client(f.key.clone());
    let client2 = f.client(f.key.clone());

    let counter1 = f.create_counter(&client1);
    for i in 0..5 {
        counter1.increase_by(i).unwrap();
    }

    if exp.creator_sync {
        assert!(counter1.sync().is_ok());
        assert_eq!(counter1.get_state(), DatatypeState::Subscribed);
    }

    let counter2 = f.subscribe_counter(&client2);
    let interceptor2 = f.interceptor(&client2);

    let expected_error_for_pull = exp.error.clone();
    let modify_push = exp.modify_push;

    interceptor2
        .set_before_push(move |push| {
            modify_push(push);
        })
        .set_after_pull(move |pull| {
            assert_push_pull_pack(
                pull,
                false,
                exp.checkpoint,
                expected_error_for_pull.clone(),
                0,
            );
            Ok(())
        });

    let sync_result = counter2.sync();
    if exp.error.is_some() {
        let err = sync_result.unwrap_err();
        assert!(matches!(err, DatatypeError::ServerRejected(_)));
        assert_eq!(counter2.get_state(), DatatypeState::Disabled);
    } else {
        assert!(sync_result.is_ok());
        assert_eq!(counter1.get_value(), counter2.get_value());
        assert_eq!(counter2.get_state(), DatatypeState::Subscribed);
        assert_ne!(
            counter1.get_attr().get_cuid(),
            counter2.get_attr().get_cuid()
        );
        assert_eq!(
            counter1.get_attr().get_duid(),
            counter2.get_attr().get_duid()
        );
    }
}

#[test]
#[instrument]
fn can_reject_readonly_unsubscribing_with_transactions() {
    let f = TestFixture::new(get_test_ids!());

    let client = f.client("client");
    let counter = f.create_counter(&client);
    counter.sync().unwrap();
    counter.increase().unwrap();
    counter.unsubscribe().unwrap();

    let interceptor = f.interceptor(&client);
    interceptor
        .set_before_push(push_set_readonly)
        .set_after_pull(|pull| {
            assert_eq!(pull.state, DatatypeState::Disabled);
            assert_eq!(pull.error, Some(PushPullError::ReadonlyViolation));
            Ok(())
        });

    assert!(matches!(
        counter.sync().unwrap_err(),
        DatatypeError::ReadonlyViolation
    ));
    assert_eq!(counter.get_state(), DatatypeState::Disabled);
    assert!(
        f.connectivity
            .get_local_datatype_server(&f.resource_id)
            .is_none()
    );
    assert!(client.get_datatype(counter.get_key()).is_none());
}

#[derive(Clone, Copy, Debug)]
enum SubscribedPushScenario {
    Normal,
    ReadonlyWithTransactions,
    ReadonlyNoTransactions,
    PullFromAnotherClient,
}

struct SubscribedPushExpectation {
    extra_ops: i64,
    use_subscriber: bool,
    modify_push: fn(&mut PushPullPack),
    is_readonly: bool,
    error: Option<PushPullError>,
    checkpoint: CheckPoint,
    tx_len: usize,
}

impl SubscribedPushScenario {
    fn expectation(self) -> SubscribedPushExpectation {
        match self {
            Self::Normal => SubscribedPushExpectation {
                extra_ops: 5,
                use_subscriber: false,
                modify_push: push_no_change,
                is_readonly: false,
                error: None,
                checkpoint: CheckPoint::new(10, 10),
                tx_len: 0,
            },
            Self::ReadonlyWithTransactions => SubscribedPushExpectation {
                extra_ops: 5,
                use_subscriber: false,
                modify_push: push_set_readonly,
                is_readonly: true,
                error: Some(PushPullError::ReadonlyViolation),
                checkpoint: CheckPoint::new(0, 0),
                tx_len: 0,
            },
            Self::ReadonlyNoTransactions => SubscribedPushExpectation {
                extra_ops: 0,
                use_subscriber: false,
                modify_push: push_set_readonly,
                is_readonly: true,
                error: None,
                checkpoint: CheckPoint::new(5, 5),
                tx_len: 0,
            },
            Self::PullFromAnotherClient => SubscribedPushExpectation {
                extra_ops: 3,
                use_subscriber: true,
                modify_push: push_no_change,
                is_readonly: false,
                error: None,
                checkpoint: CheckPoint::new(8, 0),
                tx_len: 3,
            },
        }
    }
}

#[rstest]
#[case::normal(SubscribedPushScenario::Normal)]
#[case::readonly_with_transactions(SubscribedPushScenario::ReadonlyWithTransactions)]
#[case::readonly_no_transactions(SubscribedPushScenario::ReadonlyNoTransactions)]
#[case::pull_from_another_client(SubscribedPushScenario::PullFromAnotherClient)]
#[instrument]
fn can_process_subscribe(#[case] scenario: SubscribedPushScenario) {
    let exp = scenario.expectation();
    let f = TestFixture::new(get_test_ids!());

    let client1 = f.client("client1");
    let counter1 = client1
        .subscribe_or_create_datatype(f.key.clone())
        .build_counter()
        .unwrap();
    for i in 0..5 {
        counter1.increase_by(i).unwrap();
    }
    assert!(counter1.sync().is_ok());
    assert_eq!(counter1.get_state(), DatatypeState::Subscribed);

    let client2;
    let counter2;
    let (target_client, target_counter) = if exp.use_subscriber {
        client2 = f.client("client2");
        counter2 = f.subscribe_counter(&client2);
        assert!(counter2.sync().is_ok());
        assert_eq!(counter2.get_state(), DatatypeState::Subscribed);

        for i in 0..exp.extra_ops {
            counter1.increase_by(i).unwrap();
        }
        assert!(counter1.sync().is_ok());
        (&client2, &counter2)
    } else {
        for i in 0..exp.extra_ops {
            counter1.increase_by(i).unwrap();
        }
        (&client1, &counter1)
    };

    let interceptor = f.interceptor(target_client);
    let expected_error_for_pull = exp.error.clone();
    let modify_push = exp.modify_push;
    interceptor
        .set_before_push(move |push| {
            modify_push(push);
        })
        .set_after_pull(move |pull| {
            assert_push_pull_pack(
                pull,
                exp.is_readonly,
                exp.checkpoint,
                expected_error_for_pull.clone(),
                exp.tx_len,
            );
            Ok(())
        });

    check_error(exp.error.as_ref(), target_counter);
}

fn check_error(expected_error: Option<&PushPullError>, counter: &Counter) {
    let sync_result = counter.sync();
    if expected_error.is_some() {
        assert!(matches!(
            sync_result.unwrap_err(),
            DatatypeError::ReadonlyViolation
        ));
        assert_eq!(counter.get_state(), DatatypeState::Disabled);
    } else {
        assert!(sync_result.is_ok());
        assert_eq!(counter.get_state(), DatatypeState::Subscribed);
    }
}

#[test]
#[instrument]
fn can_reject_type_mismatch_in_subscribed_push() {
    let f = TestFixture::new(get_test_ids!());
    let client = f.client(get_test_func_name!());
    let counter = f.create_counter(&client);
    counter.sync().unwrap();
    assert_eq!(counter.get_state(), DatatypeState::Subscribed);

    // Simulate a client sending a push with a different DataType after subscription.
    let interceptor = f.interceptor(&client);
    interceptor.set_before_push(push_set_variable_type);

    let result = counter.sync();
    assert!(matches!(
        result.unwrap_err(),
        DatatypeError::ServerRejected(_)
    ));
    assert_eq!(counter.get_state(), DatatypeState::Disabled);
}

#[test]
#[instrument]
fn can_reject_push_from_disabled_client() {
    let f = TestFixture::new(get_test_ids!());
    let client = f.client(get_test_func_name!());
    let counter = f.create_counter(&client);
    counter.sync().unwrap();
    assert_eq!(counter.get_state(), DatatypeState::Subscribed);

    // Simulate a buggy or malicious client sending a push with Disabled state.
    let interceptor = f.interceptor(&client);
    interceptor.set_before_push(push_set_disabled_state);

    let result = counter.sync();
    assert!(matches!(
        result.unwrap_err(),
        DatatypeError::ServerRejected(_)
    ));
    assert_eq!(counter.get_state(), DatatypeState::Disabled);
}
