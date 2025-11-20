//! SyncYam - Conflict-free datatypes with distributed synchronization
//!
//! SyncYam is a Rust SDK for building applications with conflict-free replicated data types (CRDTs)
//! that automatically synchronize across distributed systems.
//!
//! # Features
//!
//! - **CRDT Datatypes**: Conflict-free replicated data types ([`Counter`], with more coming)
//! - **Transaction Support**: Atomic transactions with automatic rollback on failure
//! - **Read-Only Mode**: Create read-only datatypes for observation without modification
//! - **Event Loop System**: Priority-based event processing with graceful shutdown
//! - **Enhanced Error Handling**: Structured stack traces with typed error codes
//! - **Observability**: Optional OpenTelemetry and Jaeger integration (via `tracing` feature)
//!
//! # Quick Start
//!
//! ```
//! use syncyam::Client;
//!
//! // Create a client
//! let client = Client::builder("my-collection", "my-client").build();
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
//! - `tracing` - Enables OpenTelemetry distributed tracing support

use std::fmt::Debug;

pub use datatypes::datatype_set::DatatypeSet;

pub use crate::{
    clients::client::Client,
    datatypes::{builder::DatatypeBuilder, counter::Counter, datatype::Datatype},
    errors::{
        BoxedError, clients::ClientError, connectivity::ConnectivityError, datatypes::DatatypeError,
    },
    types::datatype::{DataType, DatatypeState},
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

/// A trait for types that can be converted into a String and debugged.
///
/// This trait combines `Into<String>` and `Debug` bounds for convenience
/// in function parameters that need both string conversion and debug output.
///
/// # Note
///
/// This trait is automatically implemented for all types that satisfy
/// both `Into<String>` and `Debug`
pub trait IntoString: Into<String> + Debug {}

impl<T: Into<String> + Debug> IntoString for T {}

#[cfg(feature = "tracing")]
#[ctor::ctor]
pub fn init_tracing_subscriber() {
    use tracing::level_filters::LevelFilter;
    observability::tracing_for_test::init(LevelFilter::TRACE);
}
