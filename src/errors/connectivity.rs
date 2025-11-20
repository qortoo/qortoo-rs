use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConnectivityError {}
