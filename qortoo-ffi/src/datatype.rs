//! Datatype-agnostic FFI plumbing shared by every concrete datatype handle.
//!
//! Operations that do not depend on which datatype they act on — synchronization,
//! metadata, and handlers — are exported once here as `qortoo_datatype_*` over the
//! shared [`QortooDatatype`] handle. Every concrete handle owns one as a field, and
//! `qortoo_<type>_as_datatype` borrows it, so a binding writes those operations once
//! rather than once per datatype. Release stays per-type: the allocation is the
//! concrete handle, so freeing it must name the type that was allocated.
//!
//! Operations that must name the datatype stay per-type, because their signature
//! does: construction returns a concrete handle, transactions hand one to a foreign
//! callback, and an operation like `qortoo_counter_increase_by` is counter-specific.
//! What those share — argument validation, the construction flow with its
//! handler-userdata lifetime, the transaction callback bridge, trace-context
//! propagation, and error reporting — lives here as generic bodies over
//! [`DatatypeHandle`], so each `#[no_mangle]` wrapper stays a one-line forward.

use std::{ffi::c_char, ptr};

use qortoo::{BoxedError, ClientError, Datatype, DatatypeBuilder, DatatypeError, DatatypeSet};

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

/// Opaque handle to any datatype, carrying what they all have in common.
///
/// Every concrete handle (`QortooCounter`, `QortooVariable`) holds one of these, and
/// `qortoo_<type>_as_datatype` borrows it. That borrow is a reference to a field of
/// the concrete handle, not a second handle: it stays valid for exactly as long as
/// the concrete handle, which is what `qortoo_<type>_free` releases.
pub struct QortooDatatype {
    inner: DatatypeSet,
}

impl QortooDatatype {
    pub(crate) fn new(inner: DatatypeSet) -> Self {
        Self { inner }
    }

    /// The datatype this handle names, for a concrete handle to narrow.
    pub(crate) fn datatype_set(&self) -> &DatatypeSet {
        &self.inner
    }

    /// The datatype behind this handle, as the behavior every datatype shares.
    /// Adding a datatype adds one arm here and nothing else in this module.
    fn datatype(&self) -> &dyn Datatype {
        match &self.inner {
            DatatypeSet::Counter(counter) => counter,
            DatatypeSet::Variable(variable) => variable,
        }
    }
}

/// A concrete `qortoo_<type>_*` handle: it names one datatype and owns the
/// [`QortooDatatype`] the shared entry points take. One `impl` per handle type is all
/// the per-datatype glue the generic entry-point bodies in this module need.
pub(crate) trait DatatypeHandle: Sized + 'static {
    /// The concrete `qortoo` datatype this handle names.
    type Inner: Datatype + Into<DatatypeSet> + 'static;

    /// Name used in the `"<name> is null"` boundary error.
    const NAME: &'static str;

    /// Wraps a datatype in a handle — a freshly built one, or the transaction-scoped
    /// one handed to a foreign transaction callback.
    fn wrap(inner: Self::Inner) -> Self;

    /// Borrows the shared handle this concrete handle owns.
    fn shared(&self) -> &QortooDatatype;

    /// Borrows the datatype, or `None` if the handle names a different one.
    fn inner(&self) -> Option<&Self::Inner>;

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
                Ok(inner) => Box::into_raw(Box::new(H::wrap(inner))),
                Err(e) => {
                    set_err(err_out, client_error_code(&e), &e.to_string());
                    ptr::null_mut()
                }
            }
        })
    }
}

/// Releases a handle; the underlying datatype lives on inside the client.
///
/// This stays per-datatype because the allocation is the concrete handle: the shared
/// handle `qortoo_<type>_as_datatype` returns borrows a field of it, and releasing
/// the allocation must name the type that was allocated.
pub(crate) unsafe fn free<H>(handle: *mut H) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

/// Borrows the shared handle of a concrete handle, or null if `handle` is null.
///
/// Shared, never exclusive: a handle may be used from several threads at once (every
/// operation takes it by `*const`), so producing a `&mut` here — even briefly — would
/// alias a reference another thread is holding.
pub(crate) unsafe fn as_datatype<H: DatatypeHandle>(handle: *const H) -> *const QortooDatatype {
    match unsafe { handle.as_ref() } {
        Some(handle) => handle.shared(),
        None => ptr::null(),
    }
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

/// Null-checks a concrete handle argument and returns the datatype it names.
/// Datatype-specific operations that need `err_out` semantics (e.g.
/// `qortoo_counter_increase_by`) use this too.
pub(crate) unsafe fn datatype_ref<'a, H: DatatypeHandle>(
    handle: *const H,
    err_out: *mut QortooError,
) -> Option<&'a H::Inner> {
    let Some(handle) = (unsafe { handle.as_ref() }) else {
        unsafe { boundary_err(err_out, &format!("{} is null", H::NAME)) };
        return None;
    };
    match handle.inner() {
        Some(inner) => Some(inner),
        // Narrowing the stored `DatatypeSet` is fallible in the type system but not
        // in practice: a handle only ever holds the datatype its own constructor
        // built. Reported rather than assumed away.
        None => {
            unsafe { boundary_err(err_out, &format!("handle is not a {}", H::NAME)) };
            None
        }
    }
}

/// Null-checks a shared handle argument and returns the behavior every datatype shares.
unsafe fn shared_ref<'a>(
    datatype: *const QortooDatatype,
    err_out: *mut QortooError,
) -> Option<&'a dyn Datatype> {
    match unsafe { datatype.as_ref() } {
        Some(datatype) => Some(datatype.datatype()),
        None => {
            unsafe { boundary_err(err_out, "datatype is null") };
            None
        }
    }
}

/// Reports a rejected argument at the boundary.
unsafe fn boundary_err(err_out: *mut QortooError, message: &str) {
    unsafe { set_err(err_out, QORTOO_ERR_INVALID_ARGUMENT, message) };
}

/// Reports a `DatatypeError` through `err_out`; success leaves `err_out` cleared.
fn report_datatype_result(result: Result<(), DatatypeError>, err_out: *mut QortooError) {
    if let Err(e) = result {
        unsafe { set_err(err_out, datatype_error_code(&e), &e.to_string()) };
    }
}

// ---------------------------------------------------------------------------
// Shared entry points (any datatype handle)
// ---------------------------------------------------------------------------
//
// These take the shared handle, so a binding writes them once. Pass a concrete
// handle through `qortoo_<type>_as_datatype`.
//
// The getters take no `err_out`: a null handle returns the documented default.

/// Returns the `DatatypeState` discriminant, or -1 if `datatype` is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_datatype_get_state(datatype: *const QortooDatatype) -> i32 {
    unsafe { datatype.as_ref() }
        .map(|d| d.datatype().get_state() as i32)
        .unwrap_or(-1)
}

/// Returns the `DataType` discriminant, or -1 if `datatype` is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_datatype_get_type(datatype: *const QortooDatatype) -> i32 {
    unsafe { datatype.as_ref() }
        .map(|d| d.datatype().get_type() as i32)
        .unwrap_or(-1)
}

/// Returns the datatype key (release with `qortoo_string_free`), or null if
/// `datatype` is null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_datatype_get_key(datatype: *const QortooDatatype) -> *mut c_char {
    match unsafe { datatype.as_ref() } {
        Some(d) => to_owned_c_string(d.datatype().get_key()),
        None => ptr::null_mut(),
    }
}

/// Returns the server-side version (0 before the first sync or if `datatype` is null).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_datatype_get_server_version(
    datatype: *const QortooDatatype,
) -> u64 {
    unsafe { datatype.as_ref() }
        .map(|d| d.datatype().get_server_version())
        .unwrap_or(0)
}

/// Returns the client-side version (number of local operations).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_datatype_get_client_version(
    datatype: *const QortooDatatype,
) -> u64 {
    unsafe { datatype.as_ref() }
        .map(|d| d.datatype().get_client_version())
        .unwrap_or(0)
}

/// Returns the last synchronized client version.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_datatype_get_synced_client_version(
    datatype: *const QortooDatatype,
) -> u64 {
    unsafe { datatype.as_ref() }
        .map(|d| d.datatype().get_synced_client_version())
        .unwrap_or(0)
}

/// Blocking push/pull synchronization with the connectivity backend.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_datatype_sync(
    datatype: *const QortooDatatype,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(shared) = shared_ref(datatype, err_out) else {
                return;
            };
            report_datatype_result(shared.sync(), err_out);
        })
    }
}

/// `qortoo_datatype_sync` continuing the caller's trace via the W3C
/// `traceparent`/`tracestate` headers (both nullable; absent or malformed headers
/// fall back to no parent).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_datatype_sync_with_context(
    datatype: *const QortooDatatype,
    traceparent: *const c_char,
    tracestate: *const c_char,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(shared) = shared_ref(datatype, err_out) else {
                return;
            };
            with_remote_parent!("qortoo.sync", traceparent, tracestate, || {
                report_datatype_result(shared.sync(), err_out)
            })
        })
    }
}

/// Marks this datatype as unsubscribing (see `qortoo_client_unsubscribe_datatype`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_datatype_unsubscribe(
    datatype: *const QortooDatatype,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(shared) = shared_ref(datatype, err_out) else {
                return;
            };
            report_datatype_result(shared.unsubscribe(), err_out);
        })
    }
}

/// Registers (or replaces) a handler at `priority`. Callbacks arrive on Qortoo tokio
/// worker threads; `userdata_drop` fires exactly once when the handler is replaced or
/// unset — or immediately if `datatype` is null and the handler cannot be registered.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_datatype_set_handler(
    datatype: *const QortooDatatype,
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
    match unsafe { datatype.as_ref() } {
        Some(d) => d
            .datatype()
            .set_handler(priority, make_foreign_handler(ctx)),
        None => drop(ctx),
    }
}

/// Removes the handler at `priority`. Returns true if one was removed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_datatype_unset_handler(
    datatype: *const QortooDatatype,
    priority: usize,
) -> bool {
    match unsafe { datatype.as_ref() } {
        Some(d) => d.datatype().unset_handler(priority),
        None => false,
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
        let tx_handle = Box::into_raw(Box::new(H::wrap(tx_inner)));
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
