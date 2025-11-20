use crate::{
    DatatypeState,
    connectivity::Connectivity,
    errors::{connectivity::ConnectivityError, push_pull::ServerPushPullError},
    types::push_pull_pack::PushPullPack,
};

#[derive(Debug)]
pub struct NullConnectivity {}

impl NullConnectivity {
    pub fn new() -> Self {
        Self {}
    }

    fn push_transaction(&self, pushed: &PushPullPack, pulled: &mut PushPullPack) {
        pulled.checkpoint.cseq = pushed.checkpoint.cseq;
        pulled.checkpoint.sseq = pushed.checkpoint.cseq;
    }

    fn set_illegal_push_request(&self, pulled: &mut PushPullPack, reason: &str) {
        pulled.error = Some(ServerPushPullError::IllegalPushRequest(reason.to_owned()));
        pulled.state = DatatypeState::Disabled;
    }
}

impl Connectivity for NullConnectivity {
    fn is_realtime(&self) -> bool {
        true
    }

    fn push_and_pull(&self, pushed: &PushPullPack) -> Result<PushPullPack, ConnectivityError> {
        let mut pulled = pushed.get_pulled_stub();

        match pushed.state {
            DatatypeState::DueToCreate | DatatypeState::DueToSubscribeOrCreate => {
                if pushed.is_readonly {
                    self.set_illegal_push_request(
                        &mut pulled,
                        "readonly client cannot create datatype",
                    );
                    return Ok(pulled);
                }
                pulled.state = DatatypeState::DueToCreate;
                self.push_transaction(pushed, &mut pulled);
            }
            DatatypeState::DueToSubscribe => {
                pulled.state = DatatypeState::DueToSubscribe;
                if !pushed.transactions.is_empty() {
                    self.set_illegal_push_request(
                        &mut pulled,
                        "cannot subscribe with transactions",
                    );
                    return Ok(pulled);
                }
            }
            DatatypeState::Subscribed => {
                pulled.state = DatatypeState::Subscribed;
                self.push_transaction(pushed, &mut pulled);
            }
            DatatypeState::DueToUnsubscribe => {
                todo!()
            }
            DatatypeState::DueToDelete => {
                todo!()
            }
            DatatypeState::Disabled => {
                unreachable!("this cannot be happened")
            }
        }
        Ok(pulled)
    }
}

#[cfg(test)]
mod tests_null_connectivity {
    use std::sync::Arc;

    use crate::{
        DataType, DatatypeState,
        connectivity::{Connectivity, null_connectivity::NullConnectivity},
        datatypes::common::new_attribute,
        errors::push_pull::ServerPushPullError,
        operations::transaction::Transaction,
        types::{operation_id::OperationId, push_pull_pack::PushPullPack},
    };

    #[test]
    fn can_deal_with_edge_cases_in_null_connectivity() {
        let null_connectivity = NullConnectivity {};
        let attr = new_attribute!(DataType::Counter);

        let mut pushed1 = PushPullPack::new(&attr, DatatypeState::DueToCreate);
        pushed1.is_readonly = true;
        let res1 = null_connectivity.push_and_pull(&pushed1);
        assert!(res1.is_ok());
        let pulled1 = res1.unwrap();
        assert_eq!(
            pulled1.error.unwrap(),
            ServerPushPullError::IllegalPushRequest(String::new())
        );

        let mut pushed2 = PushPullPack::new(&attr, DatatypeState::DueToSubscribe);
        let mut op_id = OperationId::new();
        pushed2
            .transactions
            .push(Arc::new(Transaction::new(&mut op_id)));
        let res2 = null_connectivity.push_and_pull(&pushed2);
        assert!(res2.is_ok());
        let pulled2 = res2.unwrap();
        assert_eq!(
            pulled2.error.unwrap(),
            ServerPushPullError::IllegalPushRequest(String::new())
        );
    }
}
