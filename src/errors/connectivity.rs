use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ConnectivityError {
    #[error("[ConnectivityError] the demanded resource is not found")]
    ResourceNotFound,
}
