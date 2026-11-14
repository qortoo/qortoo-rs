//! Error out-parameter and C boundary constants; core errors provide their own codes.

use std::{
    ffi::{CString, c_char},
    ptr,
};

/// Error code used when a Rust panic is caught at the FFI boundary.
pub const QORTOO_ERR_INTERNAL_FFI: i32 = 999;
/// Error code used when an argument (null pointer, invalid UTF-8) is rejected at the boundary.
pub const QORTOO_ERR_INVALID_ARGUMENT: i32 = 998;

// Observability (900–), raised only by `qortoo_observability_init`/`_shutdown`.

/// `qortoo_observability_init` was already called in this process.
pub const QORTOO_ERR_OBSERVABILITY_ALREADY_INITIALIZED: i32 = 900;
/// `qortoo_observability_init` was called after `qortoo_observability_shutdown`.
pub const QORTOO_ERR_OBSERVABILITY_SHUT_DOWN: i32 = 901;
/// The global `tracing` subscriber is owned by someone else in this process.
pub const QORTOO_ERR_OBSERVABILITY_SUBSCRIBER: i32 = 902;
/// A telemetry exporter failed to start, flush, or stop.
pub const QORTOO_ERR_OBSERVABILITY_EXPORTER: i32 = 903;
/// The global `metrics` recorder is owned by someone else in this process.
pub const QORTOO_ERR_OBSERVABILITY_RECORDER: i32 = 904;
/// The observability options do not describe a valid configuration.
pub const QORTOO_ERR_OBSERVABILITY_INVALID_CONFIG: i32 = 905;
/// A previous initialization installed an irreversible global before a later step failed.
pub const QORTOO_ERR_OBSERVABILITY_PARTIALLY_INITIALIZED: i32 = 906;

/// Out-parameter carrying the result of a fallible call.
#[repr(C)]
pub struct QortooError {
    /// 0 on success; otherwise the Rust error discriminant (or 998/999, see constants).
    pub code: i32,
    /// Human-readable message, or null. Release with `qortoo_string_free`.
    pub msg: *mut c_char,
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

#[cfg(test)]
mod tests_error {
    use qortoo::ObservabilityError;

    use super::*;

    #[test]
    fn can_preserve_observability_codes_in_the_c_header() {
        let cases = [
            (
                ObservabilityError::AlreadyInitialized.code(),
                QORTOO_ERR_OBSERVABILITY_ALREADY_INITIALIZED,
            ),
            (
                ObservabilityError::ShutDown.code(),
                QORTOO_ERR_OBSERVABILITY_SHUT_DOWN,
            ),
            (
                ObservabilityError::Subscriber(String::new()).code(),
                QORTOO_ERR_OBSERVABILITY_SUBSCRIBER,
            ),
            (
                ObservabilityError::Exporter(String::new()).code(),
                QORTOO_ERR_OBSERVABILITY_EXPORTER,
            ),
            (
                ObservabilityError::Recorder(String::new()).code(),
                QORTOO_ERR_OBSERVABILITY_RECORDER,
            ),
            (
                ObservabilityError::InvalidConfig(String::new()).code(),
                QORTOO_ERR_OBSERVABILITY_INVALID_CONFIG,
            ),
            (
                ObservabilityError::PartiallyInitialized.code(),
                QORTOO_ERR_OBSERVABILITY_PARTIALLY_INITIALIZED,
            ),
        ];
        for (core, boundary) in cases {
            assert_eq!(core, boundary);
        }
    }

    #[test]
    fn can_accept_a_null_out_pointer_in_set_err_and_clear_err() {
        unsafe {
            set_err(ptr::null_mut(), 42, "ignored");
            clear_err(ptr::null_mut());
        }
    }

    #[test]
    fn can_populate_a_non_null_out_pointer_via_set_err() {
        let mut err = QortooError {
            code: 0,
            msg: ptr::null_mut(),
        };
        unsafe { set_err(&mut err, 42, "boom") };
        assert_eq!(err.code, 42);
        assert!(!err.msg.is_null());
        let msg = unsafe { CString::from_raw(err.msg) };
        assert_eq!(msg.to_str().unwrap(), "boom");
    }

    #[test]
    fn can_reset_a_non_null_out_pointer_via_clear_err() {
        let mut err = QortooError {
            code: 7,
            msg: crate::util::to_owned_c_string("stale"),
        };
        unsafe { clear_err(&mut err) };
        assert_eq!(err.code, 0);
        assert!(err.msg.is_null());
    }

    #[test]
    fn can_fall_back_to_a_null_message_on_an_embedded_nul_without_losing_the_code() {
        let mut err = QortooError {
            code: 0,
            msg: ptr::null_mut(),
        };
        unsafe { set_err(&mut err, 13, "bad\0message") };
        assert_eq!(
            err.code, 13,
            "the code must survive even though the message could not"
        );
        assert!(err.msg.is_null());
    }
}
