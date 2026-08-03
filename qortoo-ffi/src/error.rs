//! Error out-parameter type and Rust-error → FFI-code mapping.

use std::{
    ffi::{CString, c_char},
    ptr,
};

/// Error code used when a panic crosses the FFI boundary or an error variant is unknown.
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

pub(crate) fn observability_error_code(e: &qortoo::ObservabilityError) -> i32 {
    use qortoo::ObservabilityError as E;
    match e {
        E::AlreadyInitialized => QORTOO_ERR_OBSERVABILITY_ALREADY_INITIALIZED,
        E::ShutDown => QORTOO_ERR_OBSERVABILITY_SHUT_DOWN,
        E::PartiallyInitialized => QORTOO_ERR_OBSERVABILITY_PARTIALLY_INITIALIZED,
        E::Subscriber(_) => QORTOO_ERR_OBSERVABILITY_SUBSCRIBER,
        E::Exporter(_) => QORTOO_ERR_OBSERVABILITY_EXPORTER,
        E::Recorder(_) => QORTOO_ERR_OBSERVABILITY_RECORDER,
        E::InvalidConfig(_) => QORTOO_ERR_OBSERVABILITY_INVALID_CONFIG,
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

#[cfg(test)]
mod tests_error {
    use qortoo::ObservabilityError;

    use super::*;

    #[test]
    fn can_map_every_observability_error_to_a_distinct_code() {
        let codes = [
            observability_error_code(&ObservabilityError::AlreadyInitialized),
            observability_error_code(&ObservabilityError::ShutDown),
            observability_error_code(&ObservabilityError::PartiallyInitialized),
            observability_error_code(&ObservabilityError::Subscriber(String::new())),
            observability_error_code(&ObservabilityError::Exporter(String::new())),
            observability_error_code(&ObservabilityError::Recorder(String::new())),
            observability_error_code(&ObservabilityError::InvalidConfig(String::new())),
        ];
        let mut unique = codes.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), codes.len(), "error codes must not collide");
        assert!(
            codes.iter().all(|c| (900..=906).contains(c)),
            "observability codes stay in the 900 block the bindings mirror"
        );
    }
}
