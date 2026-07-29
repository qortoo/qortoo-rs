//! C ABI for the Qortoo SDK, consumed by the Go binding under `go/qortoo`.
//!
//! # Conventions
//!
//! - Every object is an opaque pointer created by a `*_new`/`*_create` function and
//!   released by the matching `*_free` function. Passing a pointer after `free` is UB.
//! - Fallible functions take a `QortooError*` out-parameter. `code == 0` means success;
//!   otherwise `code` is the `#[repr(i32)]` discriminant of the Rust error variant and
//!   `msg` (if non-null) must be released with `qortoo_string_free`.
//! - All input strings are NUL-terminated UTF-8. All returned strings are owned by the
//!   caller and must be released with `qortoo_string_free`.
//! - Callbacks may be invoked from Qortoo-owned tokio worker threads. String arguments
//!   passed to callbacks are only valid for the duration of the call.
//!
//! # Modules
//!
//! - `error` — `QortooError` out-parameter and error-code mapping
//! - `util` — boundary helpers (string arguments, panic guard, owned C strings)
//! - `handler` — foreign handler callbacks bridged into `DatatypeHandler`
//! - `local_connectivity` — in-memory `LocalConnectivity` backend handle
//! - `client` — `Client` handle and lifecycle entry points
//! - `counter` — `Counter` handle, operations, transactions, and handlers

// The safety contract (pointer validity, ownership, threading) is uniform across all
// exported functions and documented once in the module docs above.
#![allow(clippy::missing_safety_doc)]

mod client;
mod counter;
mod error;
mod handler;
mod local_connectivity;
mod util;

pub use client::*;
pub use counter::*;
pub use error::*;
pub use handler::*;
pub use local_connectivity::*;
pub use util::*;
