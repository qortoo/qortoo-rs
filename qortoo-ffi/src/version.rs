//! ABI and SDK version contract consumed by bindings before any other FFI call.
//!
//! The ABI major version covers exported symbol names, struct layout, and calling
//! convention. A binding compiled against one ABI major must refuse to run against a
//! native library reporting a different one instead of risking undefined behavior.
//! The minor version increases when symbols are added without breaking existing ones.

use std::ffi::c_char;

use crate::util::to_owned_c_string;

/// ABI major version this build implements. Bump on any breaking change to exported
/// symbols, struct layout, or calling convention.
pub const QORTOO_ABI_VERSION_MAJOR: u32 = 1;
/// ABI minor version this build implements. Bump when symbols are added without
/// breaking existing ones; reset to 0 when the major version bumps.
///
/// - 0: the shared `QortooDatatype` handle. Operations whose signature does not name
///   a datatype — synchronization, metadata, handlers — are exported once as
///   `qortoo_datatype_*` and reached through `qortoo_<type>_as_datatype`, replacing
///   the per-type copy of each.
pub const QORTOO_ABI_VERSION_MINOR: u32 = 0;

/// Returns the ABI major version this library implements. Compare against the value
/// the binding was generated for before making any other call into this library.
#[unsafe(no_mangle)]
pub extern "C" fn qortoo_abi_version_major() -> u32 {
    QORTOO_ABI_VERSION_MAJOR
}

/// Returns the ABI minor version this library implements.
#[unsafe(no_mangle)]
pub extern "C" fn qortoo_abi_version_minor() -> u32 {
    QORTOO_ABI_VERSION_MINOR
}

/// Returns this crate's SDK version (`CARGO_PKG_VERSION`, e.g. `"0.1.0"`). Distinct
/// from the ABI version: the SDK version can advance without an ABI break. Release the
/// returned string with `qortoo_string_free`.
#[unsafe(no_mangle)]
pub extern "C" fn qortoo_sdk_version() -> *mut c_char {
    to_owned_c_string(env!("CARGO_PKG_VERSION"))
}

#[cfg(test)]
mod tests_version {
    use std::ffi::CStr;

    use super::*;

    #[test]
    fn abi_version_functions_match_the_constants() {
        assert_eq!(qortoo_abi_version_major(), QORTOO_ABI_VERSION_MAJOR);
        assert_eq!(qortoo_abi_version_minor(), QORTOO_ABI_VERSION_MINOR);
    }

    #[test]
    fn sdk_version_is_a_non_empty_owned_c_string() {
        let ptr = qortoo_sdk_version();
        assert!(!ptr.is_null());
        let s = unsafe { CStr::from_ptr(ptr) }.to_str().unwrap();
        assert_eq!(s, env!("CARGO_PKG_VERSION"));
        unsafe { crate::qortoo_string_free(ptr) };
    }
}
