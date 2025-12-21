use std::{fmt::Debug, sync::Arc};

use crossbeam_channel::Sender;

use crate::{
    ConnectivityError,
    datatypes::{event_loop::Event, wired::WiredDatatype},
    types::push_pull_pack::PushPullPack,
};

pub mod local_connectivity;
pub mod local_datatype_server;
pub mod null_connectivity;

pub trait Connectivity: Send + Sync + Debug {
    fn register(&self, wired: Arc<WiredDatatype>, sender: Sender<Event>);
    fn push_and_pull(&self, ppp: &PushPullPack) -> Result<PushPullPack, ConnectivityError>;
    fn is_realtime(&self) -> bool;
}
