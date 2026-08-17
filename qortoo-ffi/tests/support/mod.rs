//! Test-only helpers shared by the qortoo-ffi integration-test binaries.
//!
//! Kept deliberately small and explicit: a test asserting an FFI ownership contract
//! must not lose track of which allocation it owns behind a helper. Each helper does
//! exactly one thing and leaves ownership visible at the call site.
//!
//! Not every binary that includes this module (via `mod support;`) uses every helper,
//! since each `tests/*.rs` file is compiled as its own crate.
#![allow(dead_code)]

use std::{
    ffi::{CStr, CString, c_char},
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

use qortoo_ffi::{
    QortooClient, QortooCounter, QortooError, QortooLocalConnectivity, qortoo_client_free,
    qortoo_counter_free, qortoo_local_connectivity_free, qortoo_string_free,
};

/// A fresh, zeroed `QortooError` ready to be passed as an out-parameter.
pub fn new_err() -> QortooError {
    QortooError {
        code: 0,
        msg: ptr::null_mut(),
    }
}

/// Releases any message and returns the code, leaving `err` cleared. Mirrors what a
/// binding does with every out-parameter after a call.
pub fn take_code(err: &mut QortooError) -> i32 {
    if !err.msg.is_null() {
        unsafe { qortoo_string_free(err.msg) };
        err.msg = ptr::null_mut();
    }
    let code = err.code;
    err.code = 0;
    code
}

/// Releases the message and returns `(code, owned message text)`.
pub fn take_code_and_message(err: &mut QortooError) -> (i32, Option<String>) {
    let msg = if err.msg.is_null() {
        None
    } else {
        let owned = unsafe { CStr::from_ptr(err.msg) }
            .to_string_lossy()
            .into_owned();
        unsafe { qortoo_string_free(err.msg) };
        err.msg = ptr::null_mut();
        Some(owned)
    };
    let code = err.code;
    err.code = 0;
    (code, msg)
}

/// Asserts success and releases any (unexpected) message.
pub fn assert_ok(err: &mut QortooError, what: &str) {
    let code = take_code(err);
    assert_eq!(code, 0, "{what} unexpectedly failed with code {code}");
}

/// Asserts failure with the exact expected code and releases the message.
pub fn assert_err(err: &mut QortooError, expected_code: i32, what: &str) {
    let code = take_code(err);
    assert_eq!(
        code, expected_code,
        "{what}: expected code {expected_code}, got {code}"
    );
}

/// Reads and frees a string returned by this library, or `None` for a null pointer.
pub fn read_and_free_string(ptr: *mut c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let owned = unsafe { CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned();
    unsafe { qortoo_string_free(ptr) };
    Some(owned)
}

/// A byte buffer that is NUL-terminated but not valid UTF-8, kept alive for the
/// duration of an FFI call that borrows its pointer.
pub struct InvalidUtf8CString(Vec<u8>);

impl InvalidUtf8CString {
    /// `0x80` alone is a stray UTF-8 continuation byte: never valid on its own,
    /// regardless of what surrounds it.
    pub fn new() -> Self {
        Self(vec![b'b', b'a', 0x80, b'd', 0])
    }

    pub fn as_ptr(&self) -> *const c_char {
        self.0.as_ptr() as *const c_char
    }
}

static UNIQUE_COUNTER: AtomicUsize = AtomicUsize::new(1);

/// Returns a name unique across the whole test run (process-wide), so parallel tests
/// sharing a connectivity backend or collection never collide on collection, alias,
/// or key.
pub fn unique_name(prefix: &str) -> String {
    let n = UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{n}", std::process::id())
}

pub fn unique_cstring(prefix: &str) -> CString {
    CString::new(unique_name(prefix)).expect("generated name never contains a NUL")
}

/// A unique, non-zero userdata value. Non-zero because `0` means "no userdata" at the
/// ABI boundary (see `handlerFromUserdata`/`goQortooOnStateChange` on the Go side).
pub fn unique_userdata() -> usize {
    UNIQUE_COUNTER.fetch_add(1, Ordering::Relaxed)
}

// ---------------------------------------------------------------------------
// RAII guards. Each wraps exactly one `*_free` call so a test cannot forget which
// function owns the release, even if it panics before reaching the end of the test.
// ---------------------------------------------------------------------------

pub struct ConnGuard(pub *mut QortooLocalConnectivity);

impl Drop for ConnGuard {
    fn drop(&mut self) {
        unsafe { qortoo_local_connectivity_free(self.0) };
    }
}

pub struct ClientGuard(pub *mut QortooClient);

impl Drop for ClientGuard {
    fn drop(&mut self) {
        unsafe { qortoo_client_free(self.0) };
    }
}

pub struct CounterGuard(pub *mut QortooCounter);

impl Drop for CounterGuard {
    fn drop(&mut self) {
        unsafe { qortoo_counter_free(self.0) };
    }
}
