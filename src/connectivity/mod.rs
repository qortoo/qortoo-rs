use std::fmt::Debug;

use crate::{ConnectivityError, types::push_pull_pack::PushPullPack};

pub mod null_connectivity;

pub trait Connectivity: Send + Sync + Debug {
    fn push_and_pull(&self, ppp: &PushPullPack) -> Result<PushPullPack, ConnectivityError>;
    fn is_realtime(&self) -> bool;
}
