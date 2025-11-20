use std::{fmt::Display, sync::Arc};

use crate::{
    DataType, DatatypeState,
    datatypes::common::Attribute,
    errors::push_pull::ServerPushPullError,
    operations::transaction::Transaction,
    types::{checkpoint::CheckPoint, uid::BoxedUid},
};

pub struct PushPullPack {
    pub collection: Box<str>,
    pub cuid: BoxedUid,
    pub duid: BoxedUid,
    pub key: Box<str>,
    pub r#type: DataType,
    pub state: DatatypeState,
    pub checkpoint: CheckPoint,
    pub safe_sseq: u64,
    pub transactions: Vec<Arc<Transaction>>,
    pub is_readonly: bool,
    pub has_snapshot: bool,
    pub error: Option<ServerPushPullError>,
}

impl PushPullPack {
    pub fn new(attr: &Attribute, state: DatatypeState) -> Self {
        Self {
            collection: attr.client_common.collection.clone(),
            cuid: attr.client_common.cuid.as_boxed_str(),
            duid: attr.duid.as_boxed_str(),
            key: attr.key.clone().into_boxed_str(),
            r#type: attr.r#type.to_owned(),
            state,
            checkpoint: CheckPoint::default(),
            safe_sseq: 0,
            transactions: Vec::new(),
            is_readonly: attr.is_readonly,
            has_snapshot: false,
            error: None,
        }
    }

    pub fn resource_id(&self) -> String {
        format!("{}/{}", self.collection, self.key)
    }

    pub fn checkpointing(&mut self, cp: &CheckPoint, safe_sseq: u64) {
        self.checkpoint.cseq = self
            .transactions
            .last()
            .map(|tx| tx.cseq())
            .unwrap_or(cp.cseq);
        self.checkpoint.sseq = cp.sseq;
        self.safe_sseq = safe_sseq;
    }

    pub fn get_pulled_stub(&self) -> PushPullPack {
        PushPullPack {
            collection: self.collection.clone(),
            cuid: self.cuid.clone(),
            duid: self.duid.clone(),
            key: self.key.clone(),
            r#type: self.r#type,
            state: self.state,
            checkpoint: CheckPoint::default(),
            safe_sseq: self.safe_sseq,
            transactions: Vec::new(),
            is_readonly: self.is_readonly,
            has_snapshot: false,
            error: None,
        }
    }
}

impl Display for PushPullPack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut attr = String::new();
        if self.is_readonly {
            attr.push_str("ro");
        } else {
            attr.push_str("rw");
        };

        if self.has_snapshot {
            attr.push_str("|sn");
        }

        if let Some(e) = self.error.as_ref() {
            attr.push_str(&format!("|{e}"));
        }

        write!(
            f,
            "[{:?}/{}/{} {}:{}:{} {} {} tx {}]",
            self.r#type,
            self.key,
            self.duid,
            self.checkpoint.sseq,
            self.checkpoint.cseq,
            self.safe_sseq,
            self.transactions.len(),
            attr,
            self.state
        )
    }
}

#[cfg(test)]
mod tests_push_pull_pack {

    use tracing::info;

    use crate::{
        DataType, DatatypeState, datatypes::common::new_attribute,
        errors::push_pull::ServerPushPullError, types::push_pull_pack::PushPullPack,
    };

    #[test]
    fn can_display_push_pull_pack() {
        let attr = new_attribute!(DataType::Counter);
        let mut ppp = PushPullPack::new(&attr, DatatypeState::DueToCreate);
        info!("{ppp}");
        ppp.error = Some(ServerPushPullError::IllegalPushRequest(
            "some error".to_owned(),
        ));
        info!("{ppp}");
        ppp.has_snapshot = true;
        info!("{ppp}");
    }
}
