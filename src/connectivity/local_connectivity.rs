use std::{
    collections::HashMap,
    fmt::{Debug, Formatter},
    sync::Arc,
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
}

impl LocalConnectivity {
    pub fn new_arc() -> Arc<Self> {
        Arc::new(Self {
            datatype_servers: RwLock::new(HashMap::new()),
        })
    }

    fn get_local_datatype_server(
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
            _ => todo!(),
        };
        Ok(pulled)
    }

    fn is_realtime(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests_local_connectivity {
    use crate::{
        Client, connectivity::local_connectivity::LocalConnectivity,
        utils::path::get_test_func_name,
    };

    #[test]
    fn can_use_local_connectivity() {
        let lc = LocalConnectivity::new_arc();
        Client::builder(get_test_func_name!(), "local_connectivity_test")
            .with_connectivity(lc)
            .build();
    }
}
