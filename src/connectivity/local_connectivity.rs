use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crossbeam_channel::Sender;
use parking_lot::RwLock;

use crate::{
    ConnectivityError, DatatypeState,
    connectivity::{Connectivity, local_datatype_server::LocalDatatypeServer},
    datatypes::{event_loop::Event, wired::WiredDatatype},
    types::{common::ResourceID, push_pull_pack::PushPullPack},
};

#[allow(dead_code)]
pub struct LocalConnectivity {
    datatype_servers: RwLock<HashMap<ResourceID, Arc<RwLock<LocalDatatypeServer>>>>,
    is_realtime: AtomicBool,
}

impl LocalConnectivity {
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self {
            datatype_servers: RwLock::new(HashMap::new()),
            is_realtime: AtomicBool::new(true),
        })
    }

    pub(crate) fn get_local_datatype_server(
        &self,
        resource_id: &str,
    ) -> Result<Arc<RwLock<LocalDatatypeServer>>, ConnectivityError> {
        let datatypes = self.datatype_servers.read();
        let local_datatype_server = datatypes
            .get(resource_id)
            .cloned()
            .ok_or(ConnectivityError::ResourceNotFound)?;
        Ok(local_datatype_server)
    }

    pub fn set_realtime(&self, tf: bool) {
        self.is_realtime.store(tf, Ordering::Relaxed);
    }
}

impl Debug for LocalConnectivity {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalConnectivity")
            // .field("datatype_servers", &self.datatype_servers)
            .finish()
    }
}

impl Connectivity for LocalConnectivity {
    fn register(&self, wired: Arc<WiredDatatype>, sender: Sender<Event>) {
        let attr = wired.attr.clone();
        let resource_id = attr.resource_id();

        let server = {
            let mut datatypes = self.datatype_servers.write();
            datatypes
                .entry(resource_id)
                .or_insert_with(|| Arc::new(RwLock::new(LocalDatatypeServer::new(&attr))))
                .clone()
        };

        server.write().insert_client_item(wired, sender);
    }

    fn push_and_pull(&self, pushed: &PushPullPack) -> Result<PushPullPack, ConnectivityError> {
        let resource_id = pushed.resource_id();

        let local_datatype_server_with_lock = self.get_local_datatype_server(&resource_id)?;
        let mut local_datatype_server = local_datatype_server_with_lock.write();
        let pulled = match pushed.state {
            DatatypeState::DueToCreate => local_datatype_server.process_due_to_create(pushed)?,
            DatatypeState::DueToSubscribe => {
                local_datatype_server.process_due_to_subscribe(pushed)?
            }
            _ => todo!(),
        };
        Ok(pulled)
    }

    fn is_realtime(&self) -> bool {
        self.is_realtime.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests_local_connectivity {
    use std::time::Duration;

    use tracing::instrument;

    use crate::{
        Client, Datatype, DatatypeState, connectivity::local_connectivity::LocalConnectivity,
        utils::path::get_test_func_name,
    };

    #[test]
    #[instrument]
    fn can_compare_manual_and_realtime_local_connectivity() {
        let lc_manual = LocalConnectivity::new_arc();
        lc_manual.set_realtime(false);
        let client_manual = Client::builder(get_test_func_name!(), "local_connectivity_test")
            .with_connectivity(lc_manual)
            .build();
        let counter_manual = client_manual
            .create_datatype("manual")
            .build_counter()
            .unwrap();

        let lc_realtime = LocalConnectivity::new_arc();
        let client_realtime = Client::builder(get_test_func_name!(), "local_connectivity_test")
            .with_connectivity(lc_realtime)
            .build();
        let counter_realtime = client_realtime
            .create_datatype("realtime")
            .build_counter()
            .unwrap();

        awaitility::at_most(Duration::from_secs(1))
            .poll_interval(Duration::from_micros(100))
            .until(|| counter_realtime.get_state() == DatatypeState::Subscribed);
        assert_ne!(counter_realtime.get_state(), counter_manual.get_state());

        counter_manual.sync();
        awaitility::at_most(Duration::from_secs(1))
            .poll_interval(Duration::from_micros(100))
            .until(|| counter_manual.get_state() == DatatypeState::Subscribed);
    }
}
