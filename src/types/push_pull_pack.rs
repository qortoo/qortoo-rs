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
    pub sub_gen: u64,
    pub duid: Duid,
    pub key: ArcStr,
    pub r#type: DataType,
    pub state: DatatypeState,
    pub checkpoint: CheckPoint,
    /// Push: client GC acknowledgement. Pull: server-issued GC coordination boundary.
    /// Candidate and reference checks are still required before physical collection.
    pub sseq_gc_safe: u64,
    pub transactions: Vec<Arc<Transaction>>,
    pub snapshot_transaction: Option<Arc<Transaction>>,
    pub is_readonly: bool,
    pub error: Option<PushPullError>,
}

impl PushPullPack {
    /// Creates a pack with a new subscription generation. Reuse it for subsequent requests.
    pub fn new(attr: &Attribute, state: DatatypeState) -> Self {
        Self::with_sub_gen(attr, state, attr.client_common.next_sub_gen())
    }

    pub fn with_sub_gen(attr: &Attribute, state: DatatypeState, sub_gen: u64) -> Self {
        Self {
            collection: attr.client_common.collection.clone(),
            cuid: attr.client_common.cuid.clone(),
            sub_gen,
            duid: attr.get_duid(),
            key: attr.key.clone(),
            r#type: attr.r#type.to_owned(),
            state,
            checkpoint: CheckPoint::default(),
            sseq_gc_safe: 0,
            transactions: Vec::new(),
            snapshot_transaction: None,
            is_readonly: attr.is_readonly,
            error: None,
        }
    }

    pub(crate) fn with_error(mut self, error: PushPullError) -> Self {
        self.error = Some(error);
        self.state = DatatypeState::Disabled;
        self
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

    pub fn checkpointing(&mut self, cp: &CheckPoint) {
        let cseq = self
            .transactions
            .last()
            .map(|tx| tx.cseq)
            .unwrap_or(cp.cseq);
        self.checkpoint = CheckPoint::new(cp.sseq, cseq);
    }

    pub fn get_pulled_stub(&self) -> PushPullPack {
        PushPullPack {
            collection: self.collection.clone(),
            cuid: self.cuid.clone(),
            sub_gen: self.sub_gen,
            duid: self.duid.clone(),
            key: self.key.clone(),
            r#type: self.r#type,
            state: self.state,
            checkpoint: CheckPoint::default(),
            sseq_gc_safe: 0,
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
            "[{:?}/{}/{} {} {} tx:{} s{}:c{}:gc{} {}]",
            self.r#type,
            self.key,
            self.duid,
            self.state,
            attr,
            self.transactions.len(),
            self.checkpoint.sseq,
            self.checkpoint.cseq,
            self.sseq_gc_safe,
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
    fn can_clear_request_boundaries_when_building_a_response_stub() {
        let attr = new_attribute!(DataType::Counter);
        let mut pushed = PushPullPack::new(&attr, DatatypeState::Subscribed);
        pushed.sseq_gc_safe = u64::MAX;
        let pulled = pushed.get_pulled_stub();
        assert_eq!(pulled.sseq_gc_safe, 0);
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
