use std::{collections::HashMap, fmt::Display, sync::Arc};

use crossbeam_channel::Sender;

use crate::{
    ConnectivityError, DataType, DatatypeState,
    datatypes::{common::Attribute, event_loop::Event, wired::WiredDatatype},
    errors::push_pull::ServerPushPullError,
    operations::transaction::Transaction,
    types::{
        checkpoint::CheckPoint,
        common::ArcStr,
        push_pull_pack::PushPullPack,
        uid::{Cuid, Duid},
    },
};

pub struct LocalDatatypeServer {
    wired_map: HashMap<Cuid, Arc<WiredDatatype>>,
    sender_map: HashMap<Cuid, Sender<Event>>,
    key: ArcStr,
    r#type: DataType,
    duid: Duid,
    created: bool,
    creator: Cuid,
    sseq: u64,
    cseq_map: HashMap<Cuid, CheckPoint>,
    history: Vec<Arc<Transaction>>,
}

impl Display for LocalDatatypeServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!(
            "{} '{}' subscribed by {} clients, sseq: {} created: {}",
            self.r#type,
            self.key,
            self.wired_map.len(),
            self.sseq,
            self.created
        ))
    }
}

impl LocalDatatypeServer {
    pub fn new(attr: &Attribute) -> Self {
        Self {
            wired_map: HashMap::new(),
            sender_map: HashMap::new(),
            created: false,
            // creator is temporarily assigned; it should be reassigned when this datatype is created
            creator: attr.get_cuid(),
            sseq: 0,
            cseq_map: HashMap::new(),
            history: Vec::new(),
            key: attr.key.clone(),
            r#type: attr.r#type,
            duid: attr.get_duid(),
        }
    }

    pub fn insert_client_item(&mut self, wired: Arc<WiredDatatype>, sender: Sender<Event>) {
        self.wired_map.insert(wired.cuid(), wired.clone());
        self.sender_map.insert(wired.cuid(), sender);
    }

    pub fn push_transactions(&mut self, pushed: &PushPullPack) -> u64 {
        let client_cp = self
            .cseq_map
            .entry(pushed.cuid.clone())
            .or_insert(CheckPoint::new(0, 0));

        for tx in pushed.transactions.iter() {
            if tx.cseq <= client_cp.cseq {
                continue;
            }
            self.sseq += 1;
            let mut owned_tx = (**tx).clone();
            owned_tx.sseq = self.sseq;
            self.history.push(Arc::new(owned_tx));
            client_cp.cseq = tx.cseq;
        }
        client_cp.sseq = self.sseq;
        client_cp.cseq
    }

    pub fn process_due_to_create(
        &mut self,
        pushed: &PushPullPack,
    ) -> Result<PushPullPack, ConnectivityError> {
        let mut pulled = pushed.get_pulled_stub();
        // If already created, an error should occur,
        // but if the DUID is the same, it is considered a duplicate transmission case and is allowed.
        if self.created && self.duid != pushed.duid {
            pulled.error = Some(ServerPushPullError::FailedToCreate(
                "already exist".to_string(),
            ));
            pulled.state = DatatypeState::Disabled;
            return Ok(pulled);
        }
        if pulled.is_readonly {
            pulled.error = Some(ServerPushPullError::IllegalPushRequest(
                "readonly client cannot create datatype".to_string(),
            ));
            pulled.state = DatatypeState::Disabled;
            return Ok(pulled);
        }
        self.created = true;
        self.creator = pushed.cuid.clone();
        self.duid = pushed.duid.clone();
        let cseq = self.push_transactions(pushed);
        pulled.checkpoint.sseq = self.sseq;
        pulled.checkpoint.cseq = cseq;
        pulled.state = DatatypeState::Subscribed;
        Ok(pulled)
    }

    pub fn process_due_to_subscribe_or_create(
        &mut self,
        pushed: &PushPullPack,
    ) -> Result<PushPullPack, ConnectivityError> {
        if self.created {
            self.process_due_to_subscribe(pushed)
        } else {
            self.process_due_to_create(pushed)
        }
    }

    pub fn process_subscribe(
        &mut self,
        pushed: &PushPullPack,
    ) -> Result<PushPullPack, ConnectivityError> {
        let mut pulled = pushed.get_pulled_stub();
        if pulled.is_readonly && !pushed.transactions.is_empty() {
            pulled.error = Some(ServerPushPullError::IllegalPushRequest(
                "readonly client cannot push transactions".to_string(),
            ));
            pulled.state = DatatypeState::Disabled;
            return Ok(pulled);
        }
        let cseq = self.push_transactions(pushed);
        pulled.checkpoint.sseq = pushed.checkpoint.sseq;
        pulled.checkpoint.cseq = cseq;
        self.pull_transactions(&mut pulled);
        pulled.state = DatatypeState::Subscribed;
        Ok(pulled)
    }

    pub fn process_due_to_unsubscribe(
        &mut self,
        pushed: &PushPullPack,
    ) -> Result<PushPullPack, ConnectivityError> {
        let pulled = pushed.get_pulled_stub();
        Ok(pulled)
    }

    pub fn process_due_to_delete(
        &mut self,
        pushed: &PushPullPack,
    ) -> Result<PushPullPack, ConnectivityError> {
        let pulled = pushed.get_pulled_stub();
        Ok(pulled)
    }

    pub fn process_disabled(
        &mut self,
        pushed: &PushPullPack,
    ) -> Result<PushPullPack, ConnectivityError> {
        let pulled = pushed.get_pulled_stub();
        Ok(pulled)
    }

    pub fn process_due_to_subscribe(
        &mut self,
        pushed: &PushPullPack,
    ) -> Result<PushPullPack, ConnectivityError> {
        let mut pulled = pushed.get_pulled_stub();
        if !self.created {
            pulled.error = Some(ServerPushPullError::FailedToSubscribe(format!(
                "{} '{}' not exists",
                pushed.r#type,
                pushed.resource_id(),
            )));
            pulled.state = DatatypeState::Disabled;
            return Ok(pulled);
        }
        if self.r#type != pushed.r#type {
            pulled.error = Some(ServerPushPullError::FailedToSubscribe(format!(
                "mismatched types for '{}': pushed type-{} but existed type {}",
                pushed.resource_id(),
                pushed.r#type,
                self.r#type,
            )));
            pulled.state = DatatypeState::Disabled;
            return Ok(pulled);
        }
        if !pushed.transactions.is_empty() {
            pulled.error = Some(ServerPushPullError::IllegalPushRequest(
                "cannot push transactions when subscribing".to_string(),
            ));
            pulled.state = DatatypeState::Disabled;
            return Ok(pulled);
        }

        pulled.duid = self.duid.clone();
        let wired_of_creator = self
            .get_creator_wired_datatype()
            .ok_or(ConnectivityError::ResourceNotFound(pushed.resource_id()))?;
        let tx = wired_of_creator.get_subscribe_snapshot();
        pulled.checkpoint.sseq = tx.sseq;
        pulled.snapshot_transaction = Some(Arc::new(tx));
        self.pull_transactions(&mut pulled);
        pulled.state = DatatypeState::Subscribed;
        Ok(pulled)
    }

    fn get_creator_wired_datatype(&self) -> Option<Arc<WiredDatatype>> {
        self.wired_map.get(&self.creator).cloned()
    }

    pub fn pull_transactions(&self, pulled: &mut PushPullPack) {
        let from_sseq = pulled.checkpoint.sseq;
        for tx in &self.history {
            if tx.sseq > from_sseq {
                pulled.transactions.push(tx.clone());
            }
        }
        pulled.checkpoint.sseq = self.sseq;
    }

    #[cfg(test)]
    pub fn get_wired_datatype(&self, cuid: &Cuid) -> Option<Arc<WiredDatatype>> {
        self.wired_map.get(cuid).cloned()
    }
}

#[cfg(test)]
mod tests_local_datatype_server {
    use std::sync::Arc;

    use rstest::rstest;
    use tracing::{info, instrument};

    use crate::{
        Client, Counter, DataType, Datatype, DatatypeError, DatatypeState,
        connectivity::local_connectivity::LocalConnectivity,
        errors::{
            datatypes::{DatatypeAction, DatatypeErrorWithActions, EventLoopAction},
            push_pull::ServerPushPullError,
        },
        operations::transaction::Transaction,
        types::{checkpoint::CheckPoint, push_pull_pack::PushPullPack, uid::Duid},
        utils::test_utils::{get_test_collection_name, get_test_func_name, get_test_ids},
    };

    fn push_no_change(_push: &mut PushPullPack) {}
    fn push_set_readonly(push: &mut PushPullPack) {
        push.is_readonly = true;
    }
    fn push_set_new_duid(push: &mut PushPullPack) {
        push.duid = Duid::new();
    }
    fn push_set_variable_type(push: &mut PushPullPack) {
        push.r#type = DataType::Variable;
    }
    fn push_add_transaction(push: &mut PushPullPack) {
        push.transactions.push(Arc::new(Transaction::default()));
    }

    fn assert_push_pull_pack(
        pulled: &PushPullPack,
        is_readonly: bool,
        cp: CheckPoint,
        error: Option<ServerPushPullError>,
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

    fn make_create_error() -> DatatypeErrorWithActions {
        DatatypeErrorWithActions::new(
            DatatypeError::FailedToCreate("".to_owned()),
            EventLoopAction::Normal,
            DatatypeAction::Normal,
        )
    }

    #[rstest]
    #[case::readonly(
        false,
        push_set_readonly,
        true,
        Some(ServerPushPullError::IllegalPushRequest("".to_string())),
        CheckPoint::new(0, 0),
        false,
        0,
    )]
    #[case::normal(false, push_no_change, false, None, CheckPoint::new(10, 10), true, 10)]
    #[case::duplicate(true, push_no_change, false, None, CheckPoint::new(10, 10), true, 10)]
    #[case::already_created(
        true,
        push_set_new_duid,
        false,
        Some(ServerPushPullError::FailedToCreate("already exist".to_string())),
        CheckPoint::new(0, 0),
        true,
        10,
    )]
    #[instrument]
    fn can_process_due_to_create(
        #[case] pre_create: bool,
        #[case] modify_push: fn(&mut PushPullPack),
        #[case] expected_is_readonly: bool,
        #[case] expected_error: Option<ServerPushPullError>,
        #[case] expected_cp: CheckPoint,
        #[case] expected_created: bool,
        #[case] expected_history_len: usize,
    ) {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let resource_id = format!("{}/{}", get_test_collection_name!(), get_test_func_name!());
        let client1 = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();
        for i in 0..10 {
            counter1.increase_by(i).unwrap();
        }

        let server = connectivity
            .get_local_datatype_server(&resource_id)
            .unwrap();
        let wired_interceptor1 = connectivity
            .get_wired_interceptor(&resource_id, &client1.get_cuid())
            .unwrap();

        if pre_create {
            wired_interceptor1
                .set_before_push(|_push| {})
                .set_after_pull(|_pull| Err(make_create_error()));
            let _ = counter1.sync();
            assert!(server.read().created);
        }

        let expected_error = Arc::new(expected_error);
        let expected_error2 = expected_error.clone();

        wired_interceptor1
            .set_before_push(move |push| {
                modify_push(push);
                info!("PUSH:{push}");
            })
            .set_after_pull(move |pull| {
                info!("PULL:{pull}");
                assert_push_pull_pack(
                    pull,
                    expected_is_readonly,
                    expected_cp,
                    (*expected_error2).clone(),
                    0,
                );
                Ok(())
            });

        let sync_result = counter1.sync();
        if expected_error.is_some() {
            assert!(matches!(
                sync_result.unwrap_err(),
                DatatypeError::FailedToCreate(_)
            ));
            // assert!(equal_errors!(&sync_result.unwrap_err(), &expected_error.unwrap()));
            assert_eq!(counter1.get_state(), DatatypeState::Disabled);
        } else {
            assert!(sync_result.is_ok());
            assert_eq!(counter1.get_state(), DatatypeState::Subscribed);
        }
        assert_eq!(server.read().created, expected_created);
        assert_eq!(server.read().history.len(), expected_history_len);
        info!("{}", server.read());
    }

    #[rstest]
    #[case::success(true, push_no_change, None, CheckPoint::new(5, 0))]
    #[case::not_created(
        false,
        push_no_change,
        Some(ServerPushPullError::FailedToSubscribe("".to_string())),
        CheckPoint::new(0, 0),
    )]
    #[case::type_mismatch(
        true,
        push_set_variable_type,
        Some(ServerPushPullError::FailedToSubscribe("".to_string())),
        CheckPoint::new(0, 0),
    )]
    #[case::with_transactions(
        true,
        push_add_transaction,
        Some(ServerPushPullError::IllegalPushRequest("".to_string())),
        CheckPoint::new(0, 0),
    )]
    #[instrument]
    fn can_process_due_to_subscribe(
        #[case] creator_sync: bool,
        #[case] modify_push: fn(&mut PushPullPack),
        #[case] expected_error: Option<ServerPushPullError>,
        #[case] expected_cp: CheckPoint,
    ) {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let resource_id = format!("{}/{}", get_test_collection_name!(), get_test_func_name!());

        let client1 = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let client2 = Client::builder(get_test_collection_name!(), get_test_func_name!())
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();

        let counter1 = client1
            .create_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();
        for i in 0..5 {
            counter1.increase_by(i).unwrap();
        }

        if creator_sync {
            assert!(counter1.sync().is_ok());
            assert_eq!(counter1.get_state(), DatatypeState::Subscribed);
        }

        let counter2 = client2
            .subscribe_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();
        let interceptor2 = connectivity
            .get_wired_interceptor(&resource_id, &client2.get_cuid())
            .unwrap();

        let expected_error = Arc::new(expected_error);
        let expected_error2 = expected_error.clone();

        interceptor2
            .set_before_push(move |push| {
                modify_push(push);
                info!("PUSH: {push}");
            })
            .set_after_pull(move |pull| {
                info!("PULL: {pull}");
                assert_push_pull_pack(pull, false, expected_cp, (*expected_error2).clone(), 0);
                Ok(())
            });

        let sync_result = counter2.sync();
        if expected_error.is_some() {
            assert!(matches!(
                sync_result.unwrap_err(),
                DatatypeError::FailedToSubscribe(_)
            ));
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

    #[rstest]
    #[case::normal(5, false, push_no_change, false, None, CheckPoint::new(10, 10), 5)]
    #[case::readonly_with_transactions(
        5,
        false,
        push_set_readonly,
        true,
        Some(ServerPushPullError::IllegalPushRequest("".to_string())),
        CheckPoint::new(0, 0),
        0,
    )]
    #[case::readonly_no_transactions(
        0,
        false,
        push_set_readonly,
        true,
        None,
        CheckPoint::new(5, 5),
        0
    )]
    #[case::pull_from_another_client(
        3,
        true,
        push_no_change,
        false,
        None,
        CheckPoint::new(8, 0),
        3
    )]
    #[instrument]
    fn can_process_subscribe(
        #[case] extra_ops: i64,
        #[case] use_subscriber: bool,
        #[case] modify_push: fn(&mut PushPullPack),
        #[case] expected_is_readonly: bool,
        #[case] expected_error: Option<ServerPushPullError>,
        #[case] expected_cp: CheckPoint,
        #[case] expected_tx_len: usize,
    ) {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        let (collection, key, resource_id) = get_test_ids!();

        let client1 = Client::builder(collection.clone(), "client1")
            .with_connectivity(connectivity.clone())
            .build()
            .unwrap();
        let counter1 = client1
            .subscribe_or_create_datatype(key.clone())
            .build_counter()
            .unwrap();
        for i in 0..5 {
            counter1.increase_by(i).unwrap();
        }
        assert!(counter1.sync().is_ok());
        assert_eq!(counter1.get_state(), DatatypeState::Subscribed);

        if use_subscriber {
            let client2 = Client::builder(collection, "client2")
                .with_connectivity(connectivity.clone())
                .build()
                .unwrap();
            let counter2 = client2.subscribe_datatype(key).build_counter().unwrap();
            assert!(counter2.sync().is_ok());
            assert_eq!(counter2.get_state(), DatatypeState::Subscribed);

            for i in 0..extra_ops {
                counter1.increase_by(i).unwrap();
            }
            assert!(counter1.sync().is_ok());

            let interceptor2 = connectivity
                .get_wired_interceptor(&resource_id, &client2.get_cuid())
                .unwrap();
            let expected_error = Arc::new(expected_error);
            let expected_error2 = expected_error.clone();
            interceptor2
                .set_before_push(move |push| {
                    modify_push(push);
                })
                .set_after_pull(move |pull| {
                    assert_push_pull_pack(
                        pull,
                        expected_is_readonly,
                        expected_cp,
                        (*expected_error2).clone(),
                        expected_tx_len,
                    );
                    Ok(())
                });
            check_error(expected_error, &counter2);
        } else {
            for i in 0..extra_ops {
                counter1.increase_by(i).unwrap();
            }
            let interceptor1 = connectivity
                .get_wired_interceptor(&resource_id, &client1.get_cuid())
                .unwrap();
            let expected_error = Arc::new(expected_error);
            let expected_error2 = expected_error.clone();
            interceptor1
                .set_before_push(move |push| {
                    info!("PUSH: {push}");
                    modify_push(push);
                })
                .set_after_pull(move |pull| {
                    info!("PULL: {pull}");
                    assert_push_pull_pack(
                        pull,
                        expected_is_readonly,
                        expected_cp,
                        (*expected_error2).clone(),
                        expected_tx_len,
                    );
                    Ok(())
                });
            check_error(expected_error, &counter1);
        }
    }

    fn check_error(expected_error: Arc<Option<ServerPushPullError>>, counter: &Counter) {
        let sync_result = counter.sync();
        if expected_error.is_some() {
            assert_eq!(
                sync_result.unwrap_err(),
                DatatypeError::FailedByServerPushPullError(
                    ServerPushPullError::IllegalPushRequest("".to_string())
                )
            );
            assert_eq!(counter.get_state(), DatatypeState::Disabled);
        } else {
            assert!(sync_result.is_ok());
            assert_eq!(counter.get_state(), DatatypeState::Subscribed);
        }
    }
}
