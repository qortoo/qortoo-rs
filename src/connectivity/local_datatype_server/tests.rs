pub(super) use std::{
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

pub(super) use rstest::rstest;
pub(super) use tracing::{info, instrument};

pub(super) use super::LocalDatatypeServer;
pub(super) use crate::{
    Client, Counter, DataType, Datatype, DatatypeError, DatatypeState,
    connectivity::local_connectivity::LocalConnectivity,
    datatypes::{common::new_attribute, wired_interceptor::WiredInterceptor},
    errors::{connectivity::ConnectivityError, push_pull::PushPullError},
    operations::{Operation, transaction::Transaction},
    types::{
        checkpoint::CheckPoint,
        push_pull_pack::PushPullPack,
        uid::{Cuid, Duid},
    },
    utils::test_utils::{get_test_collection_name, get_test_func_name, get_test_ids},
};

pub(super) fn transaction(cuid: &Cuid, cseq: u64, mut operation: Operation) -> Arc<Transaction> {
    operation.set_lamport(cseq);
    let mut tx = Transaction::new(cuid, cseq);
    tx.push_operation(operation);
    Arc::new(tx)
}

pub(super) struct TestFixture {
    pub(super) connectivity: Arc<LocalConnectivity>,
    pub(super) collection: String,
    pub(super) key: String,
    pub(super) resource_id: String,
}

impl TestFixture {
    pub(super) fn new(ids: (String, String, String)) -> Self {
        let connectivity = LocalConnectivity::new_arc();
        connectivity.set_realtime(false);
        Self {
            connectivity,
            collection: ids.0,
            key: ids.1,
            resource_id: ids.2,
        }
    }

    pub(super) fn client(&self, alias: impl crate::types::common::IntoString) -> Client {
        Client::builder(self.collection.clone(), alias)
            .with_connectivity(self.connectivity.clone())
            .build()
            .unwrap()
    }

    pub(super) fn create_counter(&self, client: &Client) -> Counter {
        client
            .create_datatype(self.key.clone())
            .build_counter()
            .unwrap()
    }

    pub(super) fn subscribe_counter(&self, client: &Client) -> Counter {
        client
            .subscribe_datatype(self.key.clone())
            .build_counter()
            .unwrap()
    }

    pub(super) fn subscribe_counter_readonly(&self, client: &Client) -> Counter {
        client
            .subscribe_datatype(self.key.clone())
            .with_readonly()
            .build_counter()
            .unwrap()
    }

    pub(super) fn interceptor(&self, client: &Client) -> Arc<WiredInterceptor> {
        self.connectivity
            .get_wired_interceptor(&self.resource_id, &client.get_cuid())
            .unwrap()
    }

    pub(super) fn remove_client_subscription(&self, client: &Client) {
        self.connectivity
            .remove_client_subscription(&self.resource_id, &client.get_cuid());
    }

    pub(super) fn server(&self) -> Arc<parking_lot::RwLock<LocalDatatypeServer>> {
        self.connectivity
            .get_local_datatype_server(&self.resource_id)
            .unwrap()
    }
}

mod client_sync;
mod history;
mod protocol;
mod snapshots;
