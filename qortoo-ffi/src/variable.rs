//! `Variable` handle: construction with options, lifecycle, transactions, and handlers.
//!
//! Every datatype-agnostic body lives in the `datatype` module behind
//! [`DatatypeHandle`]; this module keeps the per-type exported symbols. The JSON
//! `set`/`get` entry points are added separately.

use std::ffi::c_char;

use qortoo::{BoxedError, ClientError, DatatypeBuilder, DatatypeError, Variable};

use crate::{
    client::QortooClient,
    datatype::{self, BuildMode, DatatypeHandle, QortooDatatypeOptions},
    error::QortooError,
    handler::{QortooOnErrorCallback, QortooOnStateChangeCallback, QortooUserdataDropCallback},
};

/// Opaque handle to a `qortoo::Variable`. Cheap to clone on the Rust side; every handle
/// shares the same underlying datatype.
pub struct QortooVariable {
    pub(crate) inner: Variable,
}

impl DatatypeHandle for QortooVariable {
    type Inner = Variable;
    const NAME: &'static str = "variable";

    fn from_inner(inner: Variable) -> Self {
        Self { inner }
    }

    fn inner(&self) -> &Variable {
        &self.inner
    }

    fn build(builder: DatatypeBuilder<'_>) -> Result<Variable, ClientError> {
        builder.build_variable()
    }

    fn run_transaction<F>(inner: &Variable, tag: String, body: F) -> Result<(), DatatypeError>
    where
        F: FnOnce(Variable) -> Result<(), BoxedError> + Send + Sync + 'static,
    {
        inner.transaction(tag, body)
    }
}

/// Transaction body. Receives a transaction-scoped variable (owned by Rust — do NOT
/// free) and the userdata given to `qortoo_variable_transaction`. Return 0 to commit,
/// non-zero to roll back.
pub type QortooVariableTxCallback =
    extern "C" fn(tx_variable: *mut QortooVariable, userdata: usize) -> i32;

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

/// Builds a variable in `Creating` state (writable). `options` may be null.
/// The handler `userdata_drop` (if provided) fires exactly once even on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_create(
    client: *const QortooClient,
    key: *const c_char,
    options: *const QortooDatatypeOptions,
    err_out: *mut QortooError,
) -> *mut QortooVariable {
    unsafe { datatype::build_datatype(client, key, options, err_out, BuildMode::Create) }
}

/// Builds a variable in `Subscribing` state (read-only until synced). `options` may be null.
/// The handler `userdata_drop` (if provided) fires exactly once even on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_subscribe(
    client: *const QortooClient,
    key: *const c_char,
    options: *const QortooDatatypeOptions,
    err_out: *mut QortooError,
) -> *mut QortooVariable {
    unsafe { datatype::build_datatype(client, key, options, err_out, BuildMode::Subscribe) }
}

/// Builds a variable in `SubscribingOrCreating` state (writable). `options` may be null.
/// The handler `userdata_drop` (if provided) fires exactly once even on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_subscribe_or_create(
    client: *const QortooClient,
    key: *const c_char,
    options: *const QortooDatatypeOptions,
    err_out: *mut QortooError,
) -> *mut QortooVariable {
    unsafe { datatype::build_datatype(client, key, options, err_out, BuildMode::SubscribeOrCreate) }
}

/// Releases this variable handle; the underlying datatype lives on inside the client.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_free(variable: *mut QortooVariable) {
    unsafe { datatype::free(variable) }
}

// ---------------------------------------------------------------------------
// Synchronization and lifecycle
// ---------------------------------------------------------------------------

/// Blocking push/pull synchronization with the connectivity backend.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_sync(
    variable: *const QortooVariable,
    err_out: *mut QortooError,
) {
    unsafe { datatype::sync(variable, err_out) }
}

/// `qortoo_variable_sync` continuing the caller's trace.
///
/// `traceparent`/`tracestate` are the W3C trace-context headers of the calling span
/// (both nullable). The sync — including the push/pull that runs on the event-loop
/// thread and the handler callbacks it dispatches — becomes a child of that span.
/// Absent or malformed headers fall back to a trace without a parent.
///
/// This is a separate entry point rather than an extension of `qortoo_variable_sync`
/// so a caller that does not propagate context pays for none of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_sync_with_context(
    variable: *const QortooVariable,
    traceparent: *const c_char,
    tracestate: *const c_char,
    err_out: *mut QortooError,
) {
    unsafe { datatype::sync_with_context(variable, traceparent, tracestate, err_out) }
}

/// Marks this datatype as unsubscribing (see `qortoo_client_unsubscribe_datatype`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_unsubscribe(
    variable: *const QortooVariable,
    err_out: *mut QortooError,
) {
    unsafe { datatype::unsubscribe(variable, err_out) }
}

/// Returns the `DatatypeState` discriminant, or -1 if `variable` is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_get_state(variable: *const QortooVariable) -> i32 {
    unsafe { datatype::get_state(variable) }
}

/// Returns the `DataType` discriminant, or -1 if `variable` is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_get_type(variable: *const QortooVariable) -> i32 {
    unsafe { datatype::get_type(variable) }
}

/// Returns the datatype key (release with `qortoo_string_free`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_get_key(variable: *const QortooVariable) -> *mut c_char {
    unsafe { datatype::get_key(variable) }
}

/// Returns the server-side version (0 before the first sync or if `variable` is null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_get_server_version(
    variable: *const QortooVariable,
) -> u64 {
    unsafe { datatype::get_server_version(variable) }
}

/// Returns the client-side version (number of local operations).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_get_client_version(
    variable: *const QortooVariable,
) -> u64 {
    unsafe { datatype::get_client_version(variable) }
}

/// Returns the last synchronized client version.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_get_synced_client_version(
    variable: *const QortooVariable,
) -> u64 {
    unsafe { datatype::get_synced_client_version(variable) }
}

// ---------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------

/// Executes `callback` atomically. The callback runs inline on the calling thread with a
/// transaction-scoped variable handle owned by Rust (do NOT free it, do NOT keep it
/// after returning). A non-zero return rolls back every operation performed inside.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_transaction(
    variable: *const QortooVariable,
    tag: *const c_char,
    callback: QortooVariableTxCallback,
    userdata: usize,
    err_out: *mut QortooError,
) {
    unsafe { datatype::transaction(variable, tag, callback, userdata, err_out) }
}

/// `qortoo_variable_transaction` continuing the caller's trace.
///
/// `traceparent`/`tracestate` carry the W3C trace context of the calling span (both
/// nullable); the commit and any sync it triggers become children of that span.
///
/// This is a separate entry point rather than an extension of
/// `qortoo_variable_transaction` so a caller that does not propagate context pays for
/// none of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_transaction_with_context(
    variable: *const QortooVariable,
    tag: *const c_char,
    traceparent: *const c_char,
    tracestate: *const c_char,
    callback: QortooVariableTxCallback,
    userdata: usize,
    err_out: *mut QortooError,
) {
    unsafe {
        datatype::transaction_with_context(
            variable,
            tag,
            traceparent,
            tracestate,
            callback,
            userdata,
            err_out,
        )
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Registers (or replaces) a handler at `priority`. Callbacks arrive on Qortoo tokio
/// worker threads; `userdata_drop` fires exactly once when the handler is replaced or
/// unset — or immediately if `variable` is null and the handler cannot be registered.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_set_handler(
    variable: *const QortooVariable,
    priority: usize,
    on_state_change: QortooOnStateChangeCallback,
    on_error: QortooOnErrorCallback,
    userdata: usize,
    userdata_drop: QortooUserdataDropCallback,
) {
    unsafe {
        datatype::set_handler(
            variable,
            priority,
            on_state_change,
            on_error,
            userdata,
            userdata_drop,
        )
    }
}

/// Removes the handler at `priority`. Returns true if one was removed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_unset_handler(
    variable: *const QortooVariable,
    priority: usize,
) -> bool {
    unsafe { datatype::unset_handler(variable, priority) }
}
