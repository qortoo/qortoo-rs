//! `Client` handle: construction, accessors, and datatype lifecycle entry points.

use std::{ffi::c_char, ptr};

use qortoo::Client;

use crate::{
    error::{
        QORTOO_ERR_INVALID_ARGUMENT, QortooError, clear_err, client_error_code,
        datatype_error_code, set_err,
    },
    local_connectivity::QortooLocalConnectivity,
    util::{cstr_arg, ffi_guard, to_owned_c_string},
};

/// Opaque handle to a `qortoo::Client`.
pub struct QortooClient {
    pub(crate) inner: Client,
}

/// Creates a client. `connectivity` may be null (no-op backend).
/// Returns null on error (see `err_out`). Release with `qortoo_client_free`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_client_new(
    collection: *const c_char,
    alias: *const c_char,
    connectivity: *const QortooLocalConnectivity,
    err_out: *mut QortooError,
) -> *mut QortooClient {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, ptr::null_mut(), || {
            let Some(collection) = cstr_arg(collection, "collection", err_out) else {
                return ptr::null_mut();
            };
            let Some(alias) = cstr_arg(alias, "alias", err_out) else {
                return ptr::null_mut();
            };
            let mut builder = Client::builder(collection, alias);
            if let Some(conn) = connectivity.as_ref() {
                builder = builder.with_connectivity(conn.inner.clone());
            }
            match builder.build() {
                Ok(client) => Box::into_raw(Box::new(QortooClient { inner: client })),
                Err(e) => {
                    set_err(err_out, client_error_code(&e), &e.to_string());
                    ptr::null_mut()
                }
            }
        })
    }
}

/// Releases the client. Datatypes built from it stay valid until their own free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_client_free(client: *mut QortooClient) {
    if !client.is_null() {
        drop(unsafe { Box::from_raw(client) });
    }
}

/// Returns the collection name (release with `qortoo_string_free`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_client_get_collection(client: *const QortooClient) -> *mut c_char {
    match unsafe { client.as_ref() } {
        Some(c) => to_owned_c_string(c.inner.get_collection()),
        None => ptr::null_mut(),
    }
}

/// Returns the client alias (release with `qortoo_string_free`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_client_get_alias(client: *const QortooClient) -> *mut c_char {
    match unsafe { client.as_ref() } {
        Some(c) => to_owned_c_string(c.inner.get_alias()),
        None => ptr::null_mut(),
    }
}

/// Marks the datatype identified by `key` as unsubscribing. With manual connectivity a
/// following `qortoo_datatype_sync` drives it to Disabled.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_client_unsubscribe_datatype(
    client: *const QortooClient,
    key: *const c_char,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(client) = client.as_ref() else {
                set_err(err_out, QORTOO_ERR_INVALID_ARGUMENT, "client is null");
                return;
            };
            let Some(key) = cstr_arg(key, "key", err_out) else {
                return;
            };
            if let Err(e) = client.inner.unsubscribe_datatype(&key) {
                set_err(err_out, datatype_error_code(&e), &e.to_string());
            }
        })
    }
}
