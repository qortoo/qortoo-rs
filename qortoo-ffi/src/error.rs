//! Error out-parameter type and Rust-error → FFI-code mapping.

use std::{
    ffi::{CString, c_char},
    ptr,
};

/// Error code used when a panic crosses the FFI boundary or an error variant is unknown.
pub const QORTOO_ERR_INTERNAL_FFI: i32 = 999;
/// Error code used when an argument (null pointer, invalid UTF-8) is rejected at the boundary.
pub const QORTOO_ERR_INVALID_ARGUMENT: i32 = 998;

/// Out-parameter carrying the result of a fallible call.
#[repr(C)]
pub struct QortooError {
    /// 0 on success; otherwise the Rust error discriminant (or 998/999, see constants).
    pub code: i32,
    /// Human-readable message, or null. Release with `qortoo_string_free`.
    pub msg: *mut c_char,
}

pub(crate) fn datatype_error_code(e: &qortoo::DatatypeError) -> i32 {
    use qortoo::DatatypeError as E;
    match e {
        E::TransactionFailed(_) => 201,
        E::Internal(_) => 202,
        E::Disallowed(_) => 205,
        E::NotWritable(_) => 206,
        E::ReadonlyViolation => 207,
        E::SyncFailed(_) => 210,
        E::PushBufferExceededMaxMemSize => 211,
        E::ServerRejected(_) => 213,
        _ => QORTOO_ERR_INTERNAL_FFI,
    }
}

pub(crate) fn client_error_code(e: &qortoo::ClientError) -> i32 {
    use qortoo::ClientError as E;
    match e {
        E::InvalidCollectionName(_) => 100,
        E::FailedToSubscribeOrCreateDatatype(_) => 101,
        _ => QORTOO_ERR_INTERNAL_FFI,
    }
}

pub(crate) unsafe fn clear_err(err_out: *mut QortooError) {
    if !err_out.is_null() {
        unsafe {
            (*err_out).code = 0;
            (*err_out).msg = ptr::null_mut();
        }
    }
}

pub(crate) unsafe fn set_err(err_out: *mut QortooError, code: i32, msg: &str) {
    if !err_out.is_null() {
        unsafe {
            (*err_out).code = code;
            (*err_out).msg = CString::new(msg)
                .map(CString::into_raw)
                .unwrap_or(ptr::null_mut());
        }
    }
}
