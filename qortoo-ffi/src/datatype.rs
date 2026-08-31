//! Datatype-agnostic FFI plumbing shared by every concrete datatype handle.
//!
//! Exported symbols stay per-type (e.g. `qortoo_counter_*`) so the C ABI and its
//! ownership rules remain explicit and reviewable; this module only factors out the
//! bodies that are identical across datatypes: build options, the construction flow
//! with its handler-userdata lifetime, the foreign transaction callback bridge, and
//! error reporting.

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
    util::{cstr_arg, ffi_guard},
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

/// Shared construction body of every datatype build entry point.
///
/// Validates the client/key arguments, applies `options` (including the foreign
/// handler whose `userdata_drop` must fire exactly once on every path — early
/// failures and panics included), and boxes the handle produced by `build` on
/// success. `build` finalizes the prepared builder into a concrete handle, e.g.
/// `|b| b.build_counter().map(|inner| QortooCounter { inner })`.
pub(crate) unsafe fn build_datatype<H>(
    client: *const QortooClient,
    key: *const c_char,
    options: *const QortooDatatypeOptions,
    err_out: *mut QortooError,
    mode: BuildMode,
    build: impl FnOnce(DatatypeBuilder<'_>) -> Result<H, ClientError>,
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
            match build(builder) {
                Ok(handle) => Box::into_raw(Box::new(handle)),
                Err(e) => {
                    set_err(err_out, client_error_code(&e), &e.to_string());
                    ptr::null_mut()
                }
            }
        })
    }
}

/// Null-checks a datatype handle argument, reporting `"<name> is null"` on failure.
pub(crate) unsafe fn handle_ref<'a, H>(
    handle: *const H,
    name: &str,
    err_out: *mut QortooError,
) -> Option<&'a H> {
    match unsafe { handle.as_ref() } {
        Some(h) => Some(h),
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

/// Reports a `DatatypeError` through `err_out`; success leaves `err_out` cleared.
pub(crate) unsafe fn report_datatype_result(
    result: Result<(), DatatypeError>,
    err_out: *mut QortooError,
) {
    if let Err(e) = result {
        unsafe { set_err(err_out, datatype_error_code(&e), &e.to_string()) };
    }
}

/// Bridges a foreign transaction callback into the closure a datatype `transaction`
/// method expects. The transaction-scoped `handle` is boxed for the duration of the
/// call and reclaimed here — the callback must not free it or keep it afterwards.
/// A zero return commits; non-zero rolls back.
pub(crate) fn run_foreign_tx_callback<H>(
    handle: H,
    callback: extern "C" fn(*mut H, usize) -> i32,
    userdata: usize,
) -> Result<(), BoxedError> {
    let tx_handle = Box::into_raw(Box::new(handle));
    let code = callback(tx_handle, userdata);
    drop(unsafe { Box::from_raw(tx_handle) });
    if code == 0 {
        Ok(())
    } else {
        Err(format!("aborted by foreign callback (code {code})").into())
    }
}

/// Shared body of `qortoo_<datatype>_set_handler`: registers (or replaces) a foreign
/// handler at `priority`, or — when the handle was null and `datatype` is `None` —
/// releases the foreign userdata immediately so it is still dropped exactly once.
pub(crate) fn set_foreign_handler<D: Datatype>(
    datatype: Option<&D>,
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
    match datatype {
        Some(d) => d.set_handler(priority, make_foreign_handler(ctx)),
        None => drop(ctx),
    }
}
