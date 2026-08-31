//! Boundary helpers: string arguments, panic containment, owned C strings, and
//! owned byte buffers.

use std::{
    ffi::{CStr, CString, c_char},
    panic::{AssertUnwindSafe, catch_unwind},
    ptr,
};

use crate::error::{QORTOO_ERR_INTERNAL_FFI, QORTOO_ERR_INVALID_ARGUMENT, QortooError, set_err};

/// Caller-owned byte buffer returned by this library (e.g. a JSON payload). Release it
/// exactly once with `qortoo_owned_bytes_free`.
///
/// `{data: null, len: 0}` is the "no buffer" sentinel that an output parameter holds
/// until the call succeeds; freeing it is a no-op. A successful call replaces it with a
/// non-empty buffer.
#[repr(C)]
pub struct QortooOwnedBytes {
    /// Start of the buffer, or null for the empty sentinel.
    pub data: *mut u8,
    /// Length of the buffer in bytes.
    pub len: usize,
}

impl QortooOwnedBytes {
    /// The `{null, 0}` sentinel an output parameter is initialized to before a call runs.
    pub(crate) const EMPTY: Self = Self {
        data: ptr::null_mut(),
        len: 0,
    };
}

/// Moves `bytes` into a caller-owned [`QortooOwnedBytes`]. The buffer must be returned
/// through `qortoo_owned_bytes_free` exactly once.
pub(crate) fn into_owned_bytes(bytes: impl Into<Box<[u8]>>) -> QortooOwnedBytes {
    let boxed = bytes.into();
    let len = boxed.len();
    let data = Box::into_raw(boxed) as *mut u8;
    QortooOwnedBytes { data, len }
}

/// Releases a byte buffer produced by this library. Passing the `{null, 0}` sentinel is
/// a no-op; every other value must have come from this library and must not be freed
/// twice.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_owned_bytes_free(value: QortooOwnedBytes) {
    if value.data.is_null() {
        return;
    }
    let slice = ptr::slice_from_raw_parts_mut(value.data, value.len);
    drop(unsafe { Box::from_raw(slice) });
}

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

#[cfg(test)]
mod tests_util {
    use super::*;

    fn new_err() -> QortooError {
        QortooError {
            code: 0,
            msg: ptr::null_mut(),
        }
    }

    #[test]
    fn can_accept_valid_utf8_in_cstr_arg() {
        let mut err = new_err();
        let s = CString::new("hello").unwrap();
        let got = unsafe { cstr_arg(s.as_ptr(), "arg", &mut err) };
        assert_eq!(got.as_deref(), Some("hello"));
        assert_eq!(err.code, 0);
    }

    #[test]
    fn can_reject_a_null_pointer_in_cstr_arg() {
        let mut err = new_err();
        let got = unsafe { cstr_arg(ptr::null(), "arg", &mut err) };
        assert!(got.is_none());
        assert_eq!(err.code, QORTOO_ERR_INVALID_ARGUMENT);
    }

    #[test]
    fn can_reject_invalid_utf8_in_cstr_arg() {
        let mut err = new_err();
        // A stray UTF-8 continuation byte, NUL-terminated.
        let bytes: Vec<c_char> = vec!['b' as c_char, 0x80u8 as c_char, 0];
        let got = unsafe { cstr_arg(bytes.as_ptr(), "arg", &mut err) };
        assert!(got.is_none());
        assert_eq!(err.code, QORTOO_ERR_INVALID_ARGUMENT);
    }

    #[test]
    fn can_round_trip_a_valid_string_via_to_owned_c_string() {
        let ptr = to_owned_c_string("hello");
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, "hello");
        unsafe { qortoo_string_free(ptr) };
    }

    #[test]
    fn can_return_null_for_an_embedded_nul_via_to_owned_c_string() {
        let ptr = to_owned_c_string("bad\0string");
        assert!(ptr.is_null());
    }

    #[test]
    fn can_return_the_closures_value_on_success_via_ffi_guard() {
        let mut err = new_err();
        let v = unsafe { ffi_guard(&mut err, 0, || 7) };
        assert_eq!(v, 7);
        assert_eq!(err.code, 0);
    }

    #[test]
    fn can_convert_a_panic_into_the_default_value_and_code_999_via_ffi_guard() {
        let mut err = new_err();
        let v = unsafe { ffi_guard(&mut err, -1, || -> i32 { panic!("boundary panic") }) };
        assert_eq!(v, -1, "the default value must be returned on panic");
        assert_eq!(err.code, QORTOO_ERR_INTERNAL_FFI);
        assert!(
            !err.msg.is_null(),
            "the panic must be reported with an owned message"
        );
        unsafe { qortoo_string_free(err.msg) };
    }

    #[test]
    fn can_accept_a_null_pointer_in_qortoo_string_free() {
        unsafe { qortoo_string_free(ptr::null_mut()) };
    }

    #[test]
    fn can_round_trip_a_payload_through_owned_bytes() {
        let owned = into_owned_bytes(b"null".to_vec());
        assert!(!owned.data.is_null());
        assert_eq!(owned.len, 4);
        let seen = unsafe { std::slice::from_raw_parts(owned.data, owned.len) };
        assert_eq!(seen, b"null");
        unsafe { qortoo_owned_bytes_free(owned) };
    }

    #[test]
    fn can_free_an_empty_owned_buffer() {
        let owned = into_owned_bytes(Vec::new());
        assert_eq!(owned.len, 0);
        unsafe { qortoo_owned_bytes_free(owned) };
    }

    #[test]
    fn can_free_the_owned_bytes_sentinel() {
        unsafe { qortoo_owned_bytes_free(QortooOwnedBytes::EMPTY) };
    }
}
