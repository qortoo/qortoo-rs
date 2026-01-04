use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use crate::{
    DataType, DatatypeState,
    datatypes::common::Attribute,
    errors::push_pull::ServerPushPullError,
    operations::transaction::Transaction,
    types::{
        checkpoint::CheckPoint,
        common::{ArcStr, ResourceID},
        uid::{Cuid, Duid},
    },
};

#[derive(PartialEq, Eq)]
pub struct PushPullPack {
    pub collection: ArcStr,
    pub cuid: Cuid,
    pub duid: Duid,
    pub key: ArcStr,
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
            cuid: attr.client_common.cuid.clone(),
            duid: attr.duid.clone(),
            key: attr.key.clone(),
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

    pub fn resource_id(&self) -> ResourceID {
        format!("{}/{}", self.collection, self.key)
    }

    pub fn checkpointing(&mut self, cp: &CheckPoint, safe_sseq: u64) {
        self.checkpoint.cseq = self
            .transactions
            .last()
            .map(|tx| tx.cseq)
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

    #[cfg(test)]
    pub fn add_test_transactions(&mut self, cuid: &Cuid, from_cseq: u64, tx_size: usize) {
        for cseq in from_cseq..from_cseq + tx_size as u64 {
            let tx = Transaction::new_arc_for_test(cuid, cseq);
            self.transactions.push(tx);
            self.checkpoint.cseq = cseq;
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
        }

        if self.has_snapshot {
            attr.push_str("|sn");
        }

        let mut err = String::new();
        if let Some(e) = self.error.as_ref() {
            err.push_str(&format!("err:{e}"));
        }

        write!(
            f,
            "[{:?}/{}/{} {} {} tx:{} {}:{}:{} {}]",
            self.r#type,
            self.key,
            self.duid,
            self.state,
            attr,
            self.transactions.len(),
            self.checkpoint.sseq,
            self.checkpoint.cseq,
            self.safe_sseq,
            err
        )
    }
}

impl Debug for PushPullPack {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.to_string().as_str())
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
    fn can_use_push_pull_pack() {
        let attr = new_attribute!(DataType::Counter);
        let mut ppp = PushPullPack::new(&attr, DatatypeState::DueToCreate);
        info!("{}", ppp.resource_id());
        assert_eq!(
            ppp.resource_id(),
            format!("{}/{}", attr.client_common.collection, attr.key)
        );
        assert_eq!(format!("{ppp}"), format!("{ppp:?}"));
        info!("{ppp}");
        ppp.error = Some(ServerPushPullError::IllegalPushRequest(
            "some error".to_owned(),
        ));
        assert_eq!(format!("{ppp}"), format!("{ppp:?}"));
        info!("{ppp}");
        ppp.has_snapshot = true;
        assert_eq!(format!("{ppp}"), format!("{ppp:?}"));
        info!("{ppp:?}");
    }
}
