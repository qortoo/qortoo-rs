//! `Counter` handle: construction with options, operations, transactions, and handlers.
//!
//! Datatype-agnostic operations are exported once as `qortoo_datatype_*` (reach them
//! through `qortoo_counter_as_datatype`); this module keeps the entry points whose
//! signature names the datatype and the
//! Counter-specific operations (`increase*`, `get_value`).

use std::ffi::c_char;

use qortoo::{BoxedError, ClientError, Counter, DatatypeBuilder, DatatypeError, DatatypeSet};

use crate::{
    client::QortooClient,
    datatype::{self, BuildMode, DatatypeHandle, QortooDatatype, QortooDatatypeOptions},
    error::{QortooError, clear_err, datatype_error_code, set_err},
    util::ffi_guard,
};

/// Opaque handle to a `qortoo::Counter`. Cheap to clone on the Rust side; every handle
/// shares the same underlying datatype.
pub struct QortooCounter {
    shared: QortooDatatype,
}

impl DatatypeHandle for QortooCounter {
    type Inner = Counter;
    const NAME: &'static str = "counter";

    fn wrap(inner: Counter) -> Self {
        Self {
            shared: QortooDatatype::new(inner.into()),
        }
    }

    fn shared(&self) -> &QortooDatatype {
        &self.shared
    }

    fn inner(&self) -> Option<&Counter> {
        match self.shared.datatype_set() {
            DatatypeSet::Counter(counter) => Some(counter),
            _ => None,
        }
    }

    fn build(builder: DatatypeBuilder<'_>) -> Result<Counter, ClientError> {
        builder.build_counter()
    }

    fn run_transaction<F>(inner: &Counter, tag: String, body: F) -> Result<(), DatatypeError>
    where
        F: FnOnce(Counter) -> Result<(), BoxedError> + Send + Sync + 'static,
    {
        inner.transaction(tag, body)
    }
}

/// Transaction body. Receives a transaction-scoped counter (owned by Rust — do NOT free)
/// and the userdata given to `qortoo_counter_transaction`. Return 0 to commit, non-zero
/// to roll back.
pub type QortooTxCallback = extern "C" fn(tx_counter: *mut QortooCounter, userdata: usize) -> i32;

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

/// Builds a counter in `Creating` state (writable). `options` may be null.
/// The handler `userdata_drop` (if provided) fires exactly once even on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_create(
    client: *const QortooClient,
    key: *const c_char,
    options: *const QortooDatatypeOptions,
    err_out: *mut QortooError,
) -> *mut QortooCounter {
    unsafe { datatype::build_datatype(client, key, options, err_out, BuildMode::Create) }
}

/// Builds a counter in `Subscribing` state (read-only until synced). `options` may be null.
/// The handler `userdata_drop` (if provided) fires exactly once even on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_subscribe(
    client: *const QortooClient,
    key: *const c_char,
    options: *const QortooDatatypeOptions,
    err_out: *mut QortooError,
) -> *mut QortooCounter {
    unsafe { datatype::build_datatype(client, key, options, err_out, BuildMode::Subscribe) }
}

/// Builds a counter in `SubscribingOrCreating` state (writable). `options` may be null.
/// The handler `userdata_drop` (if provided) fires exactly once even on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_subscribe_or_create(
    client: *const QortooClient,
    key: *const c_char,
    options: *const QortooDatatypeOptions,
    err_out: *mut QortooError,
) -> *mut QortooCounter {
    unsafe { datatype::build_datatype(client, key, options, err_out, BuildMode::SubscribeOrCreate) }
}

/// Releases this counter handle; the underlying datatype lives on inside the client.
/// Any shared handle obtained from `qortoo_counter_as_datatype` dies with it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_free(counter: *mut QortooCounter) {
    unsafe { datatype::free(counter) }
}

/// Returns this counter as the shared handle the `qortoo_datatype_*` entry points
/// take — those cover synchronization, metadata, and handlers.
///
/// The result borrows `counter`: it is a pointer to a field of the counter handle, valid
/// for exactly as long as that handle. It is not a second handle and is never released
/// on its own — `qortoo_counter_free` releases the allocation exactly once, and this
/// pointer dies with it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_as_datatype(
    counter: *const QortooCounter,
) -> *const QortooDatatype {
    unsafe { datatype::as_datatype(counter) }
}

// ---------------------------------------------------------------------------
// Counter operations
// ---------------------------------------------------------------------------

/// Increases the counter by `delta` (may be negative). Returns the new value, or 0 on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_increase_by(
    counter: *const QortooCounter,
    delta: i64,
    err_out: *mut QortooError,
) -> i64 {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, 0, || {
            let Some(c) = datatype::datatype_ref(counter, err_out) else {
                return 0;
            };
            match c.increase_by(delta) {
                Ok(v) => v,
                Err(e) => {
                    set_err(err_out, datatype_error_code(&e), &e.to_string());
                    0
                }
            }
        })
    }
}

/// Increases the counter by 1. Returns the new value, or 0 on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_increase(
    counter: *const QortooCounter,
    err_out: *mut QortooError,
) -> i64 {
    unsafe { qortoo_counter_increase_by(counter, 1, err_out) }
}

/// Returns the current counter value (0 if `counter` is null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_get_value(counter: *const QortooCounter) -> i64 {
    unsafe { counter.as_ref() }
        .and_then(DatatypeHandle::inner)
        .map(Counter::get_value)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------

/// Executes `callback` atomically. The callback runs inline on the calling thread with a
/// transaction-scoped counter handle owned by Rust (do NOT free it, do NOT keep it after
/// returning). A non-zero return rolls back every operation performed inside.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_transaction(
    counter: *const QortooCounter,
    tag: *const c_char,
    callback: QortooTxCallback,
    userdata: usize,
    err_out: *mut QortooError,
) {
    unsafe { datatype::transaction(counter, tag, callback, userdata, err_out) }
}

/// `qortoo_counter_transaction` continuing the caller's trace.
///
/// `traceparent`/`tracestate` carry the W3C trace context of the calling span (both
/// nullable); the commit and any sync it triggers become children of that span.
///
/// This is a separate entry point rather than an extension of
/// `qortoo_counter_transaction` so a caller that does not propagate context pays for
/// none of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_transaction_with_context(
    counter: *const QortooCounter,
    tag: *const c_char,
    traceparent: *const c_char,
    tracestate: *const c_char,
    callback: QortooTxCallback,
    userdata: usize,
    err_out: *mut QortooError,
) {
    unsafe {
        datatype::transaction_with_context(
            counter,
            tag,
            traceparent,
            tracestate,
            callback,
            userdata,
            err_out,
        )
    }
}
