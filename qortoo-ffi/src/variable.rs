//! `Variable` handle: construction with options, lifecycle, raw JSON value access,
//! transactions, and handlers.
//!
//! Every datatype-agnostic body lives in the `datatype` module behind
//! [`DatatypeHandle`]; this module keeps the per-type exported symbols plus the
//! Variable-specific `set_raw`/`get_raw` entry points.

use std::ffi::c_char;

use qortoo::{BoxedError, ClientError, DatatypeBuilder, DatatypeError, DatatypeSet, Variable};

use crate::{
    client::QortooClient,
    datatype::{self, BuildMode, DatatypeHandle, QortooDatatype, QortooDatatypeOptions},
    error::{QORTOO_ERR_INVALID_ARGUMENT, QortooError, clear_err, datatype_error_code, set_err},
    util::{QortooOwnedBytes, ffi_guard, into_owned_bytes},
};

/// Opaque handle to a `qortoo::Variable`. Cheap to clone on the Rust side; every handle
/// shares the same underlying datatype.
pub struct QortooVariable {
    shared: QortooDatatype,
}

impl DatatypeHandle for QortooVariable {
    type Inner = Variable;
    const NAME: &'static str = "variable";

    fn wrap(inner: Variable) -> Self {
        Self {
            shared: QortooDatatype::new(inner.into()),
        }
    }

    fn shared(&self) -> &QortooDatatype {
        &self.shared
    }

    fn inner(&self) -> Option<&Variable> {
        match self.shared.datatype_set() {
            DatatypeSet::Variable(variable) => Some(variable),
            _ => None,
        }
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
/// Any shared handle obtained from `qortoo_variable_as_datatype` dies with it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_free(variable: *mut QortooVariable) {
    unsafe { datatype::free(variable) }
}

/// Returns this variable as the shared handle the `qortoo_datatype_*` entry points
/// take — those cover synchronization, metadata, and handlers.
///
/// The result borrows `variable`: it is a pointer to a field of the variable handle, valid
/// for exactly as long as that handle. It is not a second handle and is never released
/// on its own — `qortoo_variable_free` releases the allocation exactly once, and this
/// pointer dies with it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_as_datatype(
    variable: *const QortooVariable,
) -> *const QortooDatatype {
    unsafe { datatype::as_datatype(variable) }
}

// ---------------------------------------------------------------------------
// Raw JSON value
// ---------------------------------------------------------------------------

/// Sets the variable to the JSON value in `len` bytes at `data` — exactly one UTF-8
/// JSON value, stored verbatim (never reparsed or reordered; see
/// `qortoo_variable_get_raw`).
///
/// On success `previous_out` receives the value the variable held just before this set
/// as a caller-owned buffer (`"null"` for the first set), to be released once with
/// `qortoo_owned_bytes_free`. On any error the variable is left unchanged and
/// `previous_out` stays `{NULL, 0}`.
///
/// Rejects a null `variable`/`data`/`previous_out` (`QORTOO_ERR_INVALID_ARGUMENT`) and
/// input that is not exactly one UTF-8 JSON value (code 214).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_set_raw(
    variable: *const QortooVariable,
    data: *const u8,
    len: usize,
    previous_out: *mut QortooOwnedBytes,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        init_owned_out(previous_out);
        ffi_guard(err_out, (), || {
            let Some(inner) = datatype::datatype_ref(variable, err_out) else {
                return;
            };
            let Some(previous_out) = require_owned_out(previous_out, "previous_out", err_out)
            else {
                return;
            };
            let Some(json) = json_bytes_arg(data, len, err_out) else {
                return;
            };
            match inner.set_raw(json) {
                Ok(previous) => *previous_out = into_owned_bytes(previous),
                Err(e) => set_err(err_out, datatype_error_code(&e), &e.to_string()),
            }
        })
    }
}

/// Writes the current value to `value_out` as a caller-owned buffer holding the stored
/// JSON bytes verbatim (`"null"` for the initial value); release it once with
/// `qortoo_owned_bytes_free`. On any error `value_out` stays `{NULL, 0}`.
///
/// Rejects a null `variable` or `value_out` (`QORTOO_ERR_INVALID_ARGUMENT`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_variable_get_raw(
    variable: *const QortooVariable,
    value_out: *mut QortooOwnedBytes,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        init_owned_out(value_out);
        ffi_guard(err_out, (), || {
            let Some(inner) = datatype::datatype_ref(variable, err_out) else {
                return;
            };
            let Some(value_out) = require_owned_out(value_out, "value_out", err_out) else {
                return;
            };
            *value_out = into_owned_bytes(inner.get_raw());
        })
    }
}

/// Initializes an owned-bytes out-parameter to the `{NULL, 0}` sentinel, if non-null.
unsafe fn init_owned_out(out: *mut QortooOwnedBytes) {
    if !out.is_null() {
        unsafe { out.write(QortooOwnedBytes::EMPTY) };
    }
}

/// Null-checks an owned-bytes out-parameter.
unsafe fn require_owned_out<'a>(
    out: *mut QortooOwnedBytes,
    name: &str,
    err_out: *mut QortooError,
) -> Option<&'a mut QortooOwnedBytes> {
    match unsafe { out.as_mut() } {
        Some(out) => Some(out),
        None => {
            unsafe {
                set_err(
                    err_out,
                    QORTOO_ERR_INVALID_ARGUMENT,
                    &format!("{name} is null"),
                )
            };
            None
        }
    }
}

/// Reads the caller's JSON bytes, rejecting a null `data` pointer.
unsafe fn json_bytes_arg<'a>(
    data: *const u8,
    len: usize,
    err_out: *mut QortooError,
) -> Option<&'a [u8]> {
    if data.is_null() {
        unsafe { set_err(err_out, QORTOO_ERR_INVALID_ARGUMENT, "data is null") };
        return None;
    }
    Some(unsafe { std::slice::from_raw_parts(data, len) })
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
