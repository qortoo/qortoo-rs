use std::{
    fmt::{Debug, Display, Formatter},
    sync::Arc,
};

use tokio::runtime::Handle;

use crate::{
    connectivity::Connectivity,
    types::uid::Cuid,
    utils::runtime::{get_or_init_runtime_handle, reserve_to_shutdown_runtime},
};

pub struct ClientCommon {
    pub collection: Box<str>,
    pub cuid: Cuid,
    pub alias: Box<str>,
    pub handle: Handle,
    pub connectivity: Arc<dyn Connectivity>,
}

impl ClientCommon {
    pub fn new_arc(
        collection: Box<str>,
        alias: Box<str>,
        connectivity: Arc<dyn Connectivity>,
    ) -> Arc<Self> {
        let cuid = Cuid::new();
        let thread_name = format!("{collection}/{alias}/{cuid}");
        Arc::new(Self {
            handle: get_or_init_runtime_handle(thread_name.as_str()),
            collection,
            alias,
            cuid,
            connectivity,
        })
    }

    #[cfg(test)]
    pub fn new_for_test(mut paths: std::collections::VecDeque<String>) -> Arc<Self> {
        use crate::connectivity::null_connectivity::NullConnectivity;

        paths.pop_back();
        let alias = paths
            .pop_back()
            .unwrap_or("collection".into())
            .into_boxed_str();
        let collection = paths.pop_back().unwrap_or("client".into()).into_boxed_str();
        Self::new_arc(collection, alias, Arc::new(NullConnectivity::new()))
    }
}

impl Display for ClientCommon {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.collection, self.alias)
    }
}

impl Debug for ClientCommon {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("collection", &self.collection)
            .field("cuid", &self.cuid.to_string())
            .field("alias", &self.alias)
            .finish()
    }
}

impl Drop for ClientCommon {
    fn drop(&mut self) {
        let thread_name = format!("{}/{}/{}", self.collection, self.alias, self.cuid);
        reserve_to_shutdown_runtime(thread_name.as_str());
    }
}

#[cfg(test)]
macro_rules! new_client_common {
    () => {{
        let paths = crate::utils::path::caller_path!();
        crate::clients::common::ClientCommon::new_for_test(paths)
    }};
}
#[cfg(test)]
pub(crate) use new_client_common;
