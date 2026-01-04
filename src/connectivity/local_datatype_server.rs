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
            sseq: 0,
            cseq_map: HashMap::new(),
            history: Vec::new(),
            key: attr.key.clone(),
            r#type: attr.r#type,
            duid: attr.duid.clone(),
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
        // 이미 생성 되었다면 에러가 발생해야 하지만, 같은 DUID인 경우는 중복 전송 케이스로 간주하여 허용한다.
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
        pulled.duid = self.duid.clone();
        self.pull_transactions();

        Ok(pulled)
    }

    pub fn pull_transactions(&self) {}
}

#[cfg(test)]
mod tests_local_datatype_server {
    use tracing::{info, instrument};

    use crate::{
        DataType, DatatypeState,
        connectivity::{Connectivity, local_connectivity::LocalConnectivity},
        datatypes::{
            common::new_attribute_with_connectivity, wired::WiredDatatype,
            wired_interceptor::WiredInterceptor,
        },
        errors::push_pull::{ClientPushPullError, ServerPushPullError},
        types::{checkpoint::CheckPoint, push_pull_pack::PushPullPack, uid::Duid},
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
        let attr = new_attribute_with_connectivity!(DataType::Counter, connectivity.clone());

        // let cuid = attr.cuid();
        let (sender, _receiver) = crossbeam_channel::unbounded();

        let wired_interceptor = WiredInterceptor::new_arc();
        let wired1 = WiredDatatype::new_arc_for_test(
            attr.clone(),
            DatatypeState::DueToCreate,
            wired_interceptor.clone(),
        );
        connectivity.register(wired1.clone(), sender);
        let server = connectivity
            .get_local_datatype_server(&attr.resource_id())
            .unwrap();

        // readonly client should fail
        wired_interceptor
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
        let _ = wired1.push_pull();
        assert!(!server.read().created);

        // normal DUE_TO_CREATE case
        wired_interceptor
            .set_before_push(|push| {
                push.add_test_transactions(&push.cuid.clone(), 1, 10);
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
        let _ = wired1.push_pull();
        assert!(server.read().created);
        assert_eq!(server.read().history.len(), 10);

        // duplicated DUE_TO_CREATE case
        wired_interceptor
            .set_before_push(|push| {
                push.add_test_transactions(&push.cuid.clone(), 1, 10);
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

        let _ = wired1.push_pull();
        assert!(server.read().created);
        assert_eq!(server.read().history.len(), 10);

        // already-created case
        wired_interceptor
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
        let _ = wired1.push_pull();
        assert!(server.read().created);
        info!("{}", server.read());
    }
}
