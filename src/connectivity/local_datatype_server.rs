use std::{collections::HashMap, sync::Arc};

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
            if tx.cseq() <= client_cp.cseq {
                continue;
            }
            self.history.push(tx.clone());
            client_cp.cseq = tx.cseq();
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
    use tracing::info;

    use crate::{
        DataType, DatatypeState,
        connectivity::local_datatype_server::LocalDatatypeServer,
        datatypes::{common::new_attribute, wired::WiredDatatype},
        errors::push_pull::ServerPushPullError,
        types::{checkpoint::CheckPoint, push_pull_pack::PushPullPack, uid::Duid},
    };

    fn assert_pulled_push_pull_pack(
        pushed: &PushPullPack,
        pulled: &PushPullPack,
        is_readonly: bool,
        cp: CheckPoint,
        state: DatatypeState,
        error: Option<ServerPushPullError>,
    ) {
        info!("Pushed: {pushed}");
        info!("Pulled: {pulled}");

        let mut expected_pulled = pushed.get_pulled_stub();
        expected_pulled.is_readonly = is_readonly;
        expected_pulled.checkpoint = cp;
        expected_pulled.state = state;
        expected_pulled.error = error;

        info!("Expect: {expected_pulled}");
        assert_eq!(*pulled, expected_pulled);
    }

    #[test]
    fn can_process_due_to_create() {
        let attr = new_attribute!(DataType::Counter);

        let mut server = LocalDatatypeServer::new(&attr);

        let cuid = attr.cuid();
        let wired = WiredDatatype::new_arc_for_test(attr.clone(), DatatypeState::DueToCreate);
        let (sender, _receiver) = crossbeam_channel::unbounded();
        server.insert_client_item(wired, sender);

        // readonly client should fail
        let mut pushed = PushPullPack::new(&attr, DatatypeState::DueToCreate);
        pushed.is_readonly = true;
        let pulled = server.process_due_to_create(&pushed).unwrap();
        assert_pulled_push_pull_pack(
            &pushed,
            &pulled,
            true,
            CheckPoint::new(0, 0),
            DatatypeState::DueToCreate,
            Some(ServerPushPullError::FailedToCreate("".to_string())),
        );
        assert!(!server.created);

        // normal DUE_TO_CREATE case
        pushed.is_readonly = false;
        pushed.add_test_transactions(&cuid, 1, 10);
        let pulled = server.process_due_to_create(&pushed).unwrap();
        assert_pulled_push_pull_pack(
            &pushed,
            &pulled,
            false,
            CheckPoint::new(10, 10),
            DatatypeState::DueToCreate,
            None,
        );
        assert_eq!(server.history.len(), 10);

        // duplicated push
        let pulled = server.process_due_to_create(&pushed).unwrap();
        assert_pulled_push_pull_pack(
            &pushed,
            &pulled,
            false,
            CheckPoint::new(10, 10),
            DatatypeState::DueToCreate,
            None,
        );
        assert_eq!(server.history.len(), 10);

        // already-created case
        pushed.duid = Duid::new();
        let pulled = server.process_due_to_create(&pushed).unwrap();
        assert_pulled_push_pull_pack(
            &pushed,
            &pulled,
            false,
            CheckPoint::new(0, 0),
            DatatypeState::DueToCreate,
            Some(ServerPushPullError::FailedToCreate(
                "already exist".to_string(),
            )),
        );
    }
}
