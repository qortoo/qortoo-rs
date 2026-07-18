//! Boundary helpers: string arguments, panic containment, and owned C strings.

use std::{
    ffi::{CStr, CString, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
};

use crate::error::{QORTOO_ERR_INTERNAL_FFI, QORTOO_ERR_INVALID_ARGUMENT, QortooError, set_err};

pub(crate) unsafe fn cstr_arg(
    p: *const c_char,
    name: &str,
    err_out: *mut QortooError,
) -> Option<String> {
    if p.is_null() {
        unsafe {
            set_err(
                err_out,
                QORTOO_ERR_INVALID_ARGUMENT,
                &format!("{name} is null"),
            )
        };
        return None;
    }
    match unsafe { CStr::from_ptr(p) }.to_str() {
        Ok(s) => Some(s.to_owned()),
        Err(_) => {
            unsafe {
                set_err(
                    err_out,
                    QORTOO_ERR_INVALID_ARGUMENT,
                    &format!("{name} is not valid UTF-8"),
                )
            };
            None
        }
    }
}

pub(crate) fn to_owned_c_string(s: &str) -> *mut c_char {
    CString::new(s)
        .map(CString::into_raw)
        .unwrap_or(ptr::null_mut())
}

/// Runs `f` with panics converted into a `QortooError`, returning `default` on failure.
pub(crate) unsafe fn ffi_guard<T, F: FnOnce() -> T>(
    err_out: *mut QortooError,
    default: T,
    f: F,
) -> T {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(v) => v,
        Err(_) => {
            unsafe {
                set_err(
                    err_out,
                    QORTOO_ERR_INTERNAL_FFI,
                    "panic across FFI boundary",
                )
            };
            default
        }
    }
}

/// Releases a string returned by this library (or set into a `QortooError`).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_string_free(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}
