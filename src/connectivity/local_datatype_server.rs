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
            self.history.push(tx.clone());
            client_cp.cseq = tx.cseq;
            self.sseq += 1;
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
            return Ok(pulled);
        }
        if pulled.is_readonly {
            pulled.error = Some(ServerPushPullError::FailedToCreate(
                "readonly client cannot create datatype".to_string(),
            ));
            return Ok(pulled);
        }
        pulled.state = DatatypeState::DueToCreate;
        self.created = true;
        self.creator = pushed.cuid.clone();
        self.duid = pushed.duid.clone();
        let cseq = self.push_transactions(pushed);
        pulled.checkpoint.sseq = self.sseq;
        pulled.checkpoint.cseq = cseq;
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
            return Ok(pulled);
        }
        if self.r#type != pushed.r#type {
            pulled.error = Some(ServerPushPullError::FailedToSubscribe(format!(
                "mismatched types for '{}': pushed type-{} but existed type {}",
                pushed.resource_id(),
                pushed.r#type,
                self.r#type,
            )));
            return Ok(pulled);
        }
        if !pushed.transactions.is_empty() {
            pulled.error = Some(ServerPushPullError::IllegalPushRequest(
                "cannot push transactions when subscribing".to_string(),
            ));
            return Ok(pulled);
        }

        pulled.duid = self.duid.clone();
        let creator_wired = self
            .get_creator_wired_datatype()
            .ok_or(ConnectivityError::ResourceNotFound(pushed.resource_id()))?;
        let tx = creator_wired.get_subscribe_snapshot();
        pulled.checkpoint.sseq = tx.sseq;
        pulled.snapshot_transaction = Some(Arc::new(tx));
        self.pull_transactions();
        Ok(pulled)
    }

    fn get_creator_wired_datatype(&self) -> Option<Arc<WiredDatatype>> {
        self.wired_map.get(&self.creator).cloned()
    }

    pub fn pull_transactions(&self) {}

    #[cfg(test)]
    pub fn get_wired_datatype(&self, cuid: &Cuid) -> Option<Arc<WiredDatatype>> {
        self.wired_map.get(cuid).cloned()
    }
}

#[cfg(test)]
mod tests_local_datatype_server {

    use tracing::{info, instrument};

    use crate::{
        Client, Datatype, DatatypeState,
        connectivity::local_connectivity::LocalConnectivity,
        errors::push_pull::{ClientPushPullError, ServerPushPullError},
        types::{checkpoint::CheckPoint, push_pull_pack::PushPullPack, uid::Duid},
        utils::path::{get_test_collection_name, get_test_func_name},
    };

    fn assert_push_pull_pack(
        pulled: &PushPullPack,
        is_readonly: bool,
        cp: CheckPoint,
        state: DatatypeState,
        error: Option<ServerPushPullError>,
    ) {
        assert_eq!(pulled.is_readonly, is_readonly);
        assert_eq!(pulled.checkpoint, cp);
        assert_eq!(pulled.state, state);
        assert_eq!(pulled.error, error);
    }

    #[test]
    #[instrument]
    fn can_process_due_to_create() {
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

        // readonly client should fail
        wired_interceptor1
            .set_before_push(|push| {
                push.is_readonly = true;
                info!("PUSH:{push}");
            })
            .set_after_pull(|pull| {
                info!("PULL:{pull}");
                assert_push_pull_pack(
                    pull,
                    true,
                    CheckPoint::new(0, 0),
                    DatatypeState::DueToCreate,
                    Some(ServerPushPullError::FailedToCreate("".to_string())),
                );
                Err(ClientPushPullError::FailToGetAfter)
            });
        assert!(counter1.sync().is_err());
        assert!(!server.read().created);

        // normal DUE_TO_CREATE case
        wired_interceptor1
            .set_before_push(|push| {
                info!("PUSH:{push}");
            })
            .set_after_pull(|pull| {
                info!("PULL:{pull}");
                assert_push_pull_pack(
                    pull,
                    false,
                    CheckPoint::new(10, 10),
                    DatatypeState::DueToCreate,
                    None,
                );
                Err(ClientPushPullError::FailToGetAfter)
            });
        assert!(counter1.sync().is_err());
        assert!(server.read().created);
        assert_eq!(server.read().history.len(), 10);

        // duplicated DUE_TO_CREATE case
        wired_interceptor1
            .set_before_push(|push| {
                assert_eq!(push.state, DatatypeState::DueToCreate);
                info!("PUSH:{push}");
            })
            .set_after_pull(|pull| {
                info!("PULL:{pull}");
                assert_push_pull_pack(
                    pull,
                    false,
                    CheckPoint::new(10, 10),
                    DatatypeState::DueToCreate,
                    None,
                );
                Err(ClientPushPullError::FailToGetAfter)
            });

        assert!(counter1.sync().is_err());
        assert!(server.read().created);
        assert_eq!(server.read().history.len(), 10);

        // already-created case
        wired_interceptor1
            .set_before_push(|push| {
                push.duid = Duid::new();
                info!("PUSH:{push}");
            })
            .set_after_pull(|pull| {
                info!("PULL:{pull}");
                assert_push_pull_pack(
                    pull,
                    false,
                    CheckPoint::new(0, 0),
                    DatatypeState::DueToCreate,
                    Some(ServerPushPullError::FailedToCreate(
                        "already exist".to_string(),
                    )),
                );
                Err(ClientPushPullError::FailToGetAfter)
            });
        assert!(counter1.sync().is_err());
        assert!(server.read().created);
        info!("{}", server.read());
    }

    #[test]
    #[instrument]
    fn can_process_due_to_subscribe() {
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
        let _ = counter1.increase_by(42);
        assert!(counter1.sync().is_ok());

        let counter2 = client2
            .subscribe_datatype(get_test_func_name!())
            .build_counter()
            .unwrap();
        let interceptor2 = connectivity
            .get_wired_interceptor(&resource_id, &client2.get_cuid())
            .unwrap();
        interceptor2
            .set_before_push(|push| {
                info!("{push}");
            })
            .set_after_pull(|pull| {
                info!("{pull}");
                Ok(())
            });
        assert!(counter2.sync().is_ok());
        assert_eq!(counter1.get_value(), counter2.get_value());

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
