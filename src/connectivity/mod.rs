use std::{fmt::Debug, sync::Arc};

use crossbeam_channel::Sender;

use crate::{
    CollectionInfo,
    datatypes::{event_loop::Event, wired::WiredDatatype},
    errors::connectivity::ConnectivityError,
    types::{push_pull_pack::PushPullPack, uid::Cuid},
};

#[allow(dead_code)]
pub mod local_connectivity;
pub mod local_datatype_server;
pub mod null_connectivity;

pub trait Connectivity: Send + Sync + Debug {
    /// Opens a client identity for a collection and returns the collection information.
    /// Retries may reuse the same collection and CUID.
    fn open_client(
        &self,
        collection: &str,
        cuid: &Cuid,
    ) -> Result<Arc<CollectionInfo>, ConnectivityError>;
    fn register(&self, wired: Arc<WiredDatatype>, sender: Sender<Event>);
    /// Exchanges progress and transactions. Shared backends own subscription snapshots,
    /// including their accepted server sequence and clock, independently of client state.
    fn push_pull(&self, ppp: &PushPullPack) -> Result<PushPullPack, ConnectivityError>;
    fn is_realtime(&self) -> bool;
}
