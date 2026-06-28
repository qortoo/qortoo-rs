//! Qortoo - Conflict-free datatypes with distributed synchronization
//!
//! Qortoo is a Rust SDK for building applications with conflict-free replicated data types (CRDTs)
//! that automatically synchronize across distributed systems.
//!
//! # Features
//!
//! - **CRDT Datatypes**: Conflict-free replicated data types ([`Counter`], with more coming)
//! - **Transaction Support**: Atomic transactions with automatic rollback on failure
//! - **Read-Only Mode**: Create read-only datatypes for observation without modification
//! - **Event Loop System**: Priority-based event processing with graceful shutdown
//! - **Enhanced Error Handling**: Structured stack traces with typed error codes
//! - **Observability**: `tracing` instrumentation plus application-owned trace, log,
//!   metrics, and profiling exporters
//!
//! # Quick Start
//!
//! ```
//! use qortoo::Client;
//!
//! // Create a client
//! let client = Client::builder("my-collection", "my-client").build().unwrap();
//!
//! // Create and use a counter
//! let counter = client
//!     .create_datatype("my-counter")
//!     .build_counter()
//!     .unwrap();
//!
//! counter.increase().unwrap();
//! assert_eq!(counter.get_value(), 1);
//!
//! // Create a read-only counter
//! let readonly_counter = client
//!     .subscribe_datatype("observed-counter")
//!     .with_readonly()
//!     .build_counter()
//!     .unwrap();
//!
//! // Write operations fail on read-only datatypes
//! assert!(readonly_counter.increase().is_err());
//! ```
//!
//! # Architecture
//!
//! The main entry point is the [`Client`], which manages datatypes within a collection.
//! Use [`DatatypeBuilder`] to configure and create specific datatype instances.
//!
//! # Error Handling
//!
//! Operations return [`Result`] types with structured errors:
//! - [`ClientError`] - Client-level errors (datatype management)
//! - [`DatatypeError`] - Datatype operation errors (with typed error codes)
//!
//! # Feature Flags
//!
//! - `log_layer` - Exports Qortoo's local stdout `tracing_subscriber` layer

#[cfg(feature = "log_layer")]
pub use crate::observability::log_layer::QortooLogLayer;
pub use crate::{
    clients::client::Client,
    connectivity::local_connectivity::LocalConnectivity,
    datatypes::{
        builder::DatatypeBuilder, counter::Counter, datatype::Datatype, datatype_set::DatatypeSet,
        handler::DatatypeHandler,
    },
    errors::{
        BoxedError,
        clients::ClientError,
        connectivity::ConnectivityError,
        datatypes::{DatatypeError, ServerRejectReason},
    },
    types::{
        common::IntoString,
        datatype::{DataType, DatatypeState},
    },
};

pub(crate) mod clients;
pub(crate) mod connectivity;
mod constants;
pub(crate) mod datatypes;
pub(crate) mod defaults;
pub(crate) mod errors;
pub(crate) mod observability;
pub(crate) mod operations;
pub(crate) mod types;
pub(crate) mod utils;
