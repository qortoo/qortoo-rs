//! `Counter` handle: construction with options, operations, transactions, and handlers.
//!
//! Datatype-agnostic bodies live in the `datatype` module; this module keeps the
//! per-type exported symbols and everything Counter-specific (`increase*`, values).

use std::{ffi::c_char, ptr};

use qortoo::{Counter, Datatype};

use crate::{
    client::QortooClient,
    datatype::{
        BuildMode, QortooDatatypeOptions, build_datatype, handle_ref, report_datatype_result,
        run_foreign_tx_callback, set_foreign_handler,
    },
    error::{QortooError, clear_err, datatype_error_code, set_err},
    handler::{QortooOnErrorCallback, QortooOnStateChangeCallback, QortooUserdataDropCallback},
    observability::with_remote_parent,
    util::{cstr_arg, ffi_guard, to_owned_c_string},
};

/// Opaque handle to a `qortoo::Counter`. Cheap to clone on the Rust side; every handle
/// shares the same underlying datatype.
pub struct QortooCounter {
    pub(crate) inner: Counter,
}

/// Transaction body. Receives a transaction-scoped counter (owned by Rust — do NOT free)
/// and the userdata given to `qortoo_counter_transaction`. Return 0 to commit, non-zero
/// to roll back.
pub type QortooTxCallback = extern "C" fn(tx_counter: *mut QortooCounter, userdata: usize) -> i32;

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

unsafe fn build_counter(
    client: *const QortooClient,
    key: *const c_char,
    options: *const QortooDatatypeOptions,
    err_out: *mut QortooError,
    mode: BuildMode,
) -> *mut QortooCounter {
    unsafe {
        build_datatype(client, key, options, err_out, mode, |builder| {
            builder.build_counter().map(|inner| QortooCounter { inner })
        })
    }
}

/// Builds a counter in `Creating` state (writable). `options` may be null.
/// The handler `userdata_drop` (if provided) fires exactly once even on failure.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_create(
    client: *const QortooClient,
    key: *const c_char,
    options: *const QortooDatatypeOptions,
    err_out: *mut QortooError,
) -> *mut QortooCounter {
    unsafe { build_counter(client, key, options, err_out, BuildMode::Create) }
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
    unsafe { build_counter(client, key, options, err_out, BuildMode::Subscribe) }
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
    unsafe { build_counter(client, key, options, err_out, BuildMode::SubscribeOrCreate) }
}

// ---------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------

/// Releases this counter handle; the underlying datatype lives on inside the client.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_free(counter: *mut QortooCounter) {
    if !counter.is_null() {
        drop(unsafe { Box::from_raw(counter) });
    }
}

unsafe fn counter_ref<'a>(
    counter: *const QortooCounter,
    err_out: *mut QortooError,
) -> Option<&'a Counter> {
    unsafe { handle_ref(counter, "counter", err_out) }.map(|c| &c.inner)
}

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
            let Some(c) = counter_ref(counter, err_out) else {
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
        .map(|c| c.inner.get_value())
        .unwrap_or(0)
}

/// Blocking push/pull synchronization with the connectivity backend.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_sync(
    counter: *const QortooCounter,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(c) = counter_ref(counter, err_out) else {
                return;
            };
            report_datatype_result(c.sync(), err_out);
        })
    }
}

/// `qortoo_counter_sync` continuing the caller's trace.
///
/// `traceparent`/`tracestate` are the W3C trace-context headers of the calling span
/// (both nullable). The sync — including the push/pull that runs on the event-loop
/// thread and the handler callbacks it dispatches — becomes a child of that span.
/// Absent or malformed headers fall back to a trace without a parent.
///
/// This is a separate entry point rather than an extension of `qortoo_counter_sync`
/// so a caller that does not propagate context pays for none of it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_sync_with_context(
    counter: *const QortooCounter,
    traceparent: *const c_char,
    tracestate: *const c_char,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(c) = counter_ref(counter, err_out) else {
                return;
            };
            with_remote_parent!("qortoo.sync", traceparent, tracestate, || {
                report_datatype_result(c.sync(), err_out)
            })
        })
    }
}

/// Marks this datatype as unsubscribing (see `qortoo_client_unsubscribe_datatype`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_unsubscribe(
    counter: *const QortooCounter,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(c) = counter_ref(counter, err_out) else {
                return;
            };
            report_datatype_result(c.unsubscribe(), err_out);
        })
    }
}

/// Returns the `DatatypeState` discriminant, or -1 if `counter` is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_get_state(counter: *const QortooCounter) -> i32 {
    unsafe { counter.as_ref() }
        .map(|c| c.inner.get_state() as i32)
        .unwrap_or(-1)
}

/// Returns the `DataType` discriminant, or -1 if `counter` is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_get_type(counter: *const QortooCounter) -> i32 {
    unsafe { counter.as_ref() }
        .map(|c| c.inner.get_type() as i32)
        .unwrap_or(-1)
}

/// Returns the datatype key (release with `qortoo_string_free`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_get_key(counter: *const QortooCounter) -> *mut c_char {
    match unsafe { counter.as_ref() } {
        Some(c) => to_owned_c_string(c.inner.get_key()),
        None => ptr::null_mut(),
    }
}

/// Returns the server-side version (0 before the first sync or if `counter` is null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_get_server_version(counter: *const QortooCounter) -> u64 {
    unsafe { counter.as_ref() }
        .map(|c| c.inner.get_server_version())
        .unwrap_or(0)
}

/// Returns the client-side version (number of local operations).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_get_client_version(counter: *const QortooCounter) -> u64 {
    unsafe { counter.as_ref() }
        .map(|c| c.inner.get_client_version())
        .unwrap_or(0)
}

/// Returns the last synchronized client version.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_get_synced_client_version(
    counter: *const QortooCounter,
) -> u64 {
    unsafe { counter.as_ref() }
        .map(|c| c.inner.get_synced_client_version())
        .unwrap_or(0)
}

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
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(c) = counter_ref(counter, err_out) else {
                return;
            };
            let Some(tag) = cstr_arg(tag, "tag", err_out) else {
                return;
            };
            transaction(c, tag, callback, userdata, err_out);
        })
    }
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
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(c) = counter_ref(counter, err_out) else {
                return;
            };
            let Some(tag) = cstr_arg(tag, "tag", err_out) else {
                return;
            };
            with_remote_parent!("qortoo.transaction", traceparent, tracestate, || {
                transaction(c, tag, callback, userdata, err_out)
            })
        })
    }
}

/// Shared body of the two transaction entry points.
unsafe fn transaction(
    counter: &Counter,
    tag: String,
    callback: QortooTxCallback,
    userdata: usize,
    err_out: *mut QortooError,
) {
    let result = counter.transaction(tag, move |tx_counter| {
        run_foreign_tx_callback(QortooCounter { inner: tx_counter }, callback, userdata)
    });
    unsafe { report_datatype_result(result, err_out) };
}

/// Registers (or replaces) a handler at `priority`. Callbacks arrive on Qortoo tokio
/// worker threads; `userdata_drop` fires exactly once when the handler is replaced or
/// unset — or immediately if `counter` is null and the handler cannot be registered.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_set_handler(
    counter: *const QortooCounter,
    priority: usize,
    on_state_change: QortooOnStateChangeCallback,
    on_error: QortooOnErrorCallback,
    userdata: usize,
    userdata_drop: QortooUserdataDropCallback,
) {
    set_foreign_handler(
        unsafe { counter.as_ref() }.map(|c| &c.inner),
        priority,
        on_state_change,
        on_error,
        userdata,
        userdata_drop,
    );
}

/// Removes the handler at `priority`. Returns true if one was removed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_counter_unset_handler(
    counter: *const QortooCounter,
    priority: usize,
) -> bool {
    match unsafe { counter.as_ref() } {
        Some(c) => c.inner.unset_handler(priority).is_some(),
        None => false,
    }
}
