//! `Counter` handle: construction with options, operations, transactions, and handlers.

use std::{ffi::c_char, ptr};

use qortoo::{Counter, Datatype};

use crate::{
    client::QortooClient,
    error::{
        QORTOO_ERR_INVALID_ARGUMENT, QortooError, clear_err, client_error_code,
        datatype_error_code, set_err,
    },
    handler::{
        ForeignHandlerCtx, QortooOnErrorCallback, QortooOnStateChangeCallback,
        QortooUserdataDropCallback, make_foreign_handler,
    },
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

/// Options applied when building a datatype. A zeroed struct (or a null pointer where
/// the options parameter is nullable) selects the defaults.
#[repr(C)]
pub struct QortooDatatypeOptions {
    /// Marks the datatype read-only (`DatatypeBuilder::with_readonly`).
    pub readonly: bool,
    /// Max push-buffer size in bytes; 0 keeps the SDK default (clamped by the SDK).
    pub max_push_buffer_size: u64,
    /// Priority of the handler registered from the fields below (lower runs first).
    pub handler_priority: usize,
    /// Optional state-change callback; null to skip.
    pub on_state_change: QortooOnStateChangeCallback,
    /// Optional error callback; null to skip.
    pub on_error: QortooOnErrorCallback,
    /// Opaque integer (e.g., a Go `cgo.Handle`) forwarded to both callbacks.
    pub handler_userdata: usize,
    /// Optional destructor for `handler_userdata`; null to skip.
    pub handler_userdata_drop: QortooUserdataDropCallback,
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum BuildMode {
    Create,
    Subscribe,
    SubscribeOrCreate,
}

unsafe fn build_counter(
    client: *const QortooClient,
    key: *const c_char,
    options: *const QortooDatatypeOptions,
    err_out: *mut QortooError,
    mode: BuildMode,
) -> *mut QortooCounter {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, ptr::null_mut(), || {
            // Take ownership of the foreign handler userdata immediately: every
            // path out of this function — early failures and panics included —
            // must fire `userdata_drop` exactly once, either here via
            // `ForeignHandlerCtx::drop` or later when the registered handler is
            // dropped by the datatype.
            let mut handler_ctx = options.as_ref().map(|opts| ForeignHandlerCtx {
                on_state_change: opts.on_state_change,
                on_error: opts.on_error,
                userdata: opts.handler_userdata,
                userdata_drop: opts.handler_userdata_drop,
            });
            let Some(client) = client.as_ref() else {
                set_err(err_out, QORTOO_ERR_INVALID_ARGUMENT, "client is null");
                return ptr::null_mut();
            };
            let Some(key) = cstr_arg(key, "key", err_out) else {
                return ptr::null_mut();
            };
            let mut builder = match mode {
                BuildMode::Create => client.inner.create_datatype(key),
                BuildMode::Subscribe => client.inner.subscribe_datatype(key),
                BuildMode::SubscribeOrCreate => client.inner.subscribe_or_create_datatype(key),
            };
            if let Some(opts) = options.as_ref() {
                if opts.readonly {
                    builder = builder.with_readonly();
                }
                if opts.max_push_buffer_size > 0 {
                    builder =
                        builder.with_max_memory_size_of_push_buffer(opts.max_push_buffer_size);
                }
                if let Some(ctx) = handler_ctx.take() {
                    if ctx.on_state_change.is_some() || ctx.on_error.is_some() {
                        builder =
                            builder.with_handler(opts.handler_priority, make_foreign_handler(ctx));
                    }
                    // No callbacks: ctx drops here and releases the userdata,
                    // since Rust retains nothing that could fire it later.
                }
            }
            match builder.build_counter() {
                Ok(counter) => Box::into_raw(Box::new(QortooCounter { inner: counter })),
                Err(e) => {
                    set_err(err_out, client_error_code(&e), &e.to_string());
                    ptr::null_mut()
                }
            }
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
    match unsafe { counter.as_ref() } {
        Some(c) => Some(&c.inner),
        None => {
            unsafe { set_err(err_out, QORTOO_ERR_INVALID_ARGUMENT, "counter is null") };
            None
        }
    }
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
            if let Err(e) = c.sync() {
                set_err(err_out, datatype_error_code(&e), &e.to_string());
            }
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
            if let Err(e) = c.unsubscribe() {
                set_err(err_out, datatype_error_code(&e), &e.to_string());
            }
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
            let result = c.transaction(tag, move |tx_counter| {
                let tx_handle = Box::into_raw(Box::new(QortooCounter { inner: tx_counter }));
                let code = callback(tx_handle, userdata);
                drop(Box::from_raw(tx_handle));
                if code == 0 {
                    Ok(())
                } else {
                    Err(format!("aborted by foreign callback (code {code})").into())
                }
            });
            if let Err(e) = result {
                set_err(err_out, datatype_error_code(&e), &e.to_string());
            }
        })
    }
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
    // Constructed before the null check so the foreign userdata is released
    // exactly once even when the handler cannot be registered.
    let ctx = ForeignHandlerCtx {
        on_state_change,
        on_error,
        userdata,
        userdata_drop,
    };
    match unsafe { counter.as_ref() } {
        Some(c) => c.inner.set_handler(priority, make_foreign_handler(ctx)),
        None => drop(ctx),
    }
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
