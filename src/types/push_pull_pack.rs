use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use crate::{
    DataType, DatatypeState,
    datatypes::common::Attribute,
    errors::push_pull::PushPullError,
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
    pub snapshot_transaction: Option<Arc<Transaction>>,
    pub is_readonly: bool,
    pub error: Option<PushPullError>,
}

impl PushPullPack {
    pub fn new(attr: &Attribute, state: DatatypeState) -> Self {
        Self {
            collection: attr.client_common.collection.clone(),
            cuid: attr.client_common.cuid.clone(),
            duid: attr.get_duid(),
            key: attr.key.clone(),
            r#type: attr.r#type.to_owned(),
            state,
            checkpoint: CheckPoint::default(),
            safe_sseq: 0,
            transactions: Vec::new(),
            snapshot_transaction: None,
            is_readonly: attr.is_readonly,
            error: None,
        }
    }

    pub fn resource_id(&self) -> ResourceID {
        format!("{}/{}", self.collection, self.key)
    }

    pub(crate) fn transactions_after_cseq(
        &self,
        cseq: u64,
    ) -> impl Iterator<Item = &Arc<Transaction>> {
        self.transactions.iter().filter(move |tx| tx.cseq > cseq)
    }

    pub fn checkpointing(&mut self, cp: &CheckPoint, safe_sseq: u64) {
        let cseq = self
            .transactions
            .last()
            .map(|tx| tx.cseq)
            .unwrap_or(cp.cseq);
        self.checkpoint = CheckPoint::new(cp.sseq, cseq);
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
            snapshot_transaction: None,
            is_readonly: self.is_readonly,
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
        }

        if self.snapshot_transaction.is_some() {
            attr.push_str("|sn");
        }

        let mut err = String::new();
        if let Some(e) = self.error.as_ref() {
            err.push_str(&format!("err:{e}"));
        }

        write!(
            f,
            "[{:?}/{}/{} {} {} tx:{} s{}:c{}:sf{} {}]",
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
        errors::push_pull::PushPullError, types::push_pull_pack::PushPullPack,
    };

    #[test]
    fn can_borrow_transactions_after_a_client_sequence_without_reordering() {
        use crate::operations::transaction::Transaction;

        let attr = new_attribute!(DataType::Counter);
        let mut pack = PushPullPack::new(&attr, DatatypeState::Subscribed);
        assert!(pack.transactions_after_cseq(0).next().is_none());
        pack.transactions = [3, 1, 4, 2, 4]
            .into_iter()
            .map(|cseq| Transaction::new_arc_for_test(&pack.cuid, cseq))
            .collect();
        let selected: Vec<_> = pack.transactions_after_cseq(2).collect();
        assert_eq!(
            selected.iter().map(|tx| tx.cseq).collect::<Vec<_>>(),
            vec![3, 4, 4]
        );
        for (borrowed, index) in selected.into_iter().zip([0, 2, 4]) {
            assert!(std::ptr::eq(borrowed, &pack.transactions[index]));
        }
        assert!(pack.transactions_after_cseq(4).next().is_none());
    }

    #[test]
    fn can_use_push_pull_pack() {
        let attr = new_attribute!(DataType::Counter);
        let mut ppp = PushPullPack::new(&attr, DatatypeState::Creating);
        info!("{}", ppp.resource_id());
        assert_eq!(
            ppp.resource_id(),
            format!("{}/{}", attr.client_common.collection, attr.key)
        );
        assert_eq!(format!("{ppp}"), format!("{ppp:?}"));
        info!("{ppp}");
        ppp.error = Some(PushPullError::ProtocolViolation("some error".to_owned()));
        assert_eq!(format!("{ppp}"), format!("{ppp:?}"));
        info!("{ppp}");
        ppp.snapshot_transaction =
            Some(crate::operations::transaction::Transaction::new_arc_for_test(&ppp.cuid, 0));
        assert_eq!(format!("{ppp}"), format!("{ppp:?}"));
        info!("{ppp:?}");
    }
}
