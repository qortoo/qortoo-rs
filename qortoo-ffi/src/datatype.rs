//! Datatype-agnostic FFI plumbing shared by every concrete datatype handle.
//!
//! Every exported symbol stays per-type (e.g. `qortoo_counter_sync`,
//! `qortoo_variable_sync`) so the C ABI and its ownership rules remain explicit and
//! individually reviewable. What those symbols have in common — argument validation,
//! the construction flow with its handler-userdata lifetime, the transaction callback
//! bridge, trace-context propagation, and error reporting — lives here as generic
//! bodies over [`DatatypeHandle`], so each `#[no_mangle]` wrapper is a one-line
//! forward and a new datatype only writes the wrappers plus its own operations.

use std::{ffi::c_char, ptr};

use qortoo::{BoxedError, ClientError, Datatype, DatatypeBuilder, DatatypeError};

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
    observability::with_remote_parent,
    util::{cstr_arg, ffi_guard, to_owned_c_string},
};

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

/// Lifecycle intent of a `qortoo_<datatype>_create/subscribe/subscribe_or_create`
/// entry point, mapped onto the matching `Client` builder method.
#[derive(Clone, Copy)]
pub(crate) enum BuildMode {
    Create,
    Subscribe,
    SubscribeOrCreate,
}

/// An opaque `qortoo_<type>_*` handle: the FFI-owned newtype around a concrete
/// `qortoo` datatype. One `impl` per handle type is all the per-datatype glue the
/// generic entry-point bodies in this module need.
pub(crate) trait DatatypeHandle: Sized + 'static {
    /// The concrete `qortoo` datatype this handle wraps.
    type Inner: Datatype + 'static;

    /// Name used in the `"<name> is null"` boundary error.
    const NAME: &'static str;

    /// Wraps a datatype in a handle — a freshly built one, or the transaction-scoped
    /// one handed to a foreign transaction callback.
    fn from_inner(inner: Self::Inner) -> Self;

    /// Borrows the wrapped datatype.
    fn inner(&self) -> &Self::Inner;

    /// Finalizes a prepared builder into the concrete datatype.
    fn build(builder: DatatypeBuilder<'_>) -> Result<Self::Inner, ClientError>;

    /// Runs `body` inside the datatype's transaction, rolling back on `Err`.
    fn run_transaction<F>(inner: &Self::Inner, tag: String, body: F) -> Result<(), DatatypeError>
    where
        F: FnOnce(Self::Inner) -> Result<(), BoxedError> + Send + Sync + 'static;
}

// ---------------------------------------------------------------------------
// Construction
// ---------------------------------------------------------------------------

/// Shared construction body of every datatype build entry point.
///
/// Validates the client/key arguments, applies `options` (including the foreign
/// handler whose `userdata_drop` must fire exactly once on every path — early
/// failures and panics included), and boxes the handle on success.
pub(crate) unsafe fn build_datatype<H: DatatypeHandle>(
    client: *const QortooClient,
    key: *const c_char,
    options: *const QortooDatatypeOptions,
    err_out: *mut QortooError,
    mode: BuildMode,
) -> *mut H {
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
            match H::build(builder) {
                Ok(inner) => Box::into_raw(Box::new(H::from_inner(inner))),
                Err(e) => {
                    set_err(err_out, client_error_code(&e), &e.to_string());
                    ptr::null_mut()
                }
            }
        })
    }
}

/// Releases a handle; the underlying datatype lives on inside the client.
pub(crate) unsafe fn free<H>(handle: *mut H) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

/// Null-checks a handle argument, reporting `"<H::NAME> is null"` and returning the
/// wrapped datatype on success. Datatype-specific operations that need `err_out`
/// semantics (e.g. `qortoo_counter_increase_by`) use this too.
pub(crate) unsafe fn datatype_ref<'a, H: DatatypeHandle>(
    handle: *const H,
    err_out: *mut QortooError,
) -> Option<&'a H::Inner> {
    match unsafe { handle.as_ref() } {
        Some(h) => Some(h.inner()),
        None => {
            unsafe {
                set_err(
                    err_out,
                    QORTOO_ERR_INVALID_ARGUMENT,
                    &format!("{} is null", H::NAME),
                )
            };
            None
        }
    }
}

/// Reports a `DatatypeError` through `err_out`; success leaves `err_out` cleared.
fn report_datatype_result(result: Result<(), DatatypeError>, err_out: *mut QortooError) {
    if let Err(e) = result {
        unsafe { set_err(err_out, datatype_error_code(&e), &e.to_string()) };
    }
}

// ---------------------------------------------------------------------------
// Metadata getters (no `err_out`: a null handle returns the documented default)
// ---------------------------------------------------------------------------

/// The `DatatypeState` discriminant, or -1 if `handle` is null.
pub(crate) unsafe fn get_state<H: DatatypeHandle>(handle: *const H) -> i32 {
    unsafe { handle.as_ref() }
        .map(|h| h.inner().get_state() as i32)
        .unwrap_or(-1)
}

/// The `DataType` discriminant, or -1 if `handle` is null.
pub(crate) unsafe fn get_type<H: DatatypeHandle>(handle: *const H) -> i32 {
    unsafe { handle.as_ref() }
        .map(|h| h.inner().get_type() as i32)
        .unwrap_or(-1)
}

/// The datatype key (release with `qortoo_string_free`), or null if `handle` is null.
pub(crate) unsafe fn get_key<H: DatatypeHandle>(handle: *const H) -> *mut c_char {
    match unsafe { handle.as_ref() } {
        Some(h) => to_owned_c_string(h.inner().get_key()),
        None => ptr::null_mut(),
    }
}

/// The server-side version (0 before the first sync or if `handle` is null).
pub(crate) unsafe fn get_server_version<H: DatatypeHandle>(handle: *const H) -> u64 {
    unsafe { handle.as_ref() }
        .map(|h| h.inner().get_server_version())
        .unwrap_or(0)
}

/// The client-side version (number of local operations).
pub(crate) unsafe fn get_client_version<H: DatatypeHandle>(handle: *const H) -> u64 {
    unsafe { handle.as_ref() }
        .map(|h| h.inner().get_client_version())
        .unwrap_or(0)
}

/// The last synchronized client version.
pub(crate) unsafe fn get_synced_client_version<H: DatatypeHandle>(handle: *const H) -> u64 {
    unsafe { handle.as_ref() }
        .map(|h| h.inner().get_synced_client_version())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Synchronization and lifecycle
// ---------------------------------------------------------------------------

/// Blocking push/pull synchronization with the connectivity backend.
pub(crate) unsafe fn sync<H: DatatypeHandle>(handle: *const H, err_out: *mut QortooError) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(inner) = datatype_ref(handle, err_out) else {
                return;
            };
            report_datatype_result(inner.sync(), err_out);
        })
    }
}

/// [`sync`] continuing the caller's trace via the W3C `traceparent`/`tracestate`
/// headers (both nullable; absent or malformed headers fall back to no parent).
pub(crate) unsafe fn sync_with_context<H: DatatypeHandle>(
    handle: *const H,
    traceparent: *const c_char,
    tracestate: *const c_char,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(inner) = datatype_ref(handle, err_out) else {
                return;
            };
            with_remote_parent!("qortoo.sync", traceparent, tracestate, || {
                report_datatype_result(inner.sync(), err_out)
            })
        })
    }
}

/// Marks this datatype as unsubscribing (see `qortoo_client_unsubscribe_datatype`).
pub(crate) unsafe fn unsubscribe<H: DatatypeHandle>(handle: *const H, err_out: *mut QortooError) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(inner) = datatype_ref(handle, err_out) else {
                return;
            };
            report_datatype_result(inner.unsubscribe(), err_out);
        })
    }
}

// ---------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------

/// Runs `callback` atomically. It executes inline on the calling thread with a
/// transaction-scoped handle owned by Rust (the callback must not free it or keep it);
/// a non-zero return rolls back every operation performed inside.
pub(crate) unsafe fn transaction<H: DatatypeHandle>(
    handle: *const H,
    tag: *const c_char,
    callback: extern "C" fn(*mut H, usize) -> i32,
    userdata: usize,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(inner) = datatype_ref(handle, err_out) else {
                return;
            };
            let Some(tag) = cstr_arg(tag, "tag", err_out) else {
                return;
            };
            run_transaction(inner, tag, callback, userdata, err_out);
        })
    }
}

/// [`transaction`] continuing the caller's trace via the W3C `traceparent`/`tracestate`
/// headers (both nullable).
pub(crate) unsafe fn transaction_with_context<H: DatatypeHandle>(
    handle: *const H,
    tag: *const c_char,
    traceparent: *const c_char,
    tracestate: *const c_char,
    callback: extern "C" fn(*mut H, usize) -> i32,
    userdata: usize,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(inner) = datatype_ref(handle, err_out) else {
                return;
            };
            let Some(tag) = cstr_arg(tag, "tag", err_out) else {
                return;
            };
            with_remote_parent!("qortoo.transaction", traceparent, tracestate, || {
                run_transaction(inner, tag, callback, userdata, err_out)
            })
        })
    }
}

/// Shared body of the two transaction entry points: bridges the foreign callback into
/// the closure the datatype's `transaction` expects and reports the outcome.
fn run_transaction<H: DatatypeHandle>(
    inner: &H::Inner,
    tag: String,
    callback: extern "C" fn(*mut H, usize) -> i32,
    userdata: usize,
    err_out: *mut QortooError,
) {
    let result = H::run_transaction(inner, tag, move |tx_inner| {
        // The transaction-scoped handle is boxed for the duration of the call and
        // reclaimed here; the callback must not free it or keep it afterwards.
        let tx_handle = Box::into_raw(Box::new(H::from_inner(tx_inner)));
        let code = callback(tx_handle, userdata);
        drop(unsafe { Box::from_raw(tx_handle) });
        if code == 0 {
            Ok(())
        } else {
            Err(format!("aborted by foreign callback (code {code})").into())
        }
    });
    report_datatype_result(result, err_out);
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Registers (or replaces) a handler at `priority`. When `handle` is null the handler
/// cannot be registered, so the foreign `userdata_drop` fires immediately instead —
/// still exactly once.
pub(crate) unsafe fn set_handler<H: DatatypeHandle>(
    handle: *const H,
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
    match unsafe { handle.as_ref() } {
        Some(h) => h.inner().set_handler(priority, make_foreign_handler(ctx)),
        None => drop(ctx),
    }
}

/// Removes the handler at `priority`. Returns true if one was removed.
pub(crate) unsafe fn unset_handler<H: DatatypeHandle>(handle: *const H, priority: usize) -> bool {
    match unsafe { handle.as_ref() } {
        Some(h) => h.inner().unset_handler(priority),
        None => false,
    }
}
