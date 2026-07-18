//! In-memory `LocalConnectivity` backend handle.

use std::sync::Arc;

use qortoo::LocalConnectivity;

/// Opaque handle to a `qortoo::LocalConnectivity`. The same handle may be passed to
/// several `qortoo_client_new` calls to share one in-memory backend.
pub struct QortooLocalConnectivity {
    pub(crate) inner: Arc<LocalConnectivity>,
}

/// Creates an in-memory connectivity backend (realtime mode by default).
#[unsafe(no_mangle)]
pub extern "C" fn qortoo_local_connectivity_new() -> *mut QortooLocalConnectivity {
    Box::into_raw(Box::new(QortooLocalConnectivity {
        inner: LocalConnectivity::new_arc(),
    }))
}

/// Switches between realtime (true) and manual (false) synchronization.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_local_connectivity_set_realtime(
    conn: *mut QortooLocalConnectivity,
    realtime: bool,
) {
    if let Some(conn) = unsafe { conn.as_ref() } {
        conn.inner.set_realtime(realtime);
    }
}

/// Releases the connectivity handle. Clients created with it keep their own reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_local_connectivity_free(conn: *mut QortooLocalConnectivity) {
    if !conn.is_null() {
        drop(unsafe { Box::from_raw(conn) });
    }
}
