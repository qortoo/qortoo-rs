//! Foreign handler callbacks bridged into a `qortoo::DatatypeHandler`.

use std::{
    ffi::{CString, c_char},
    sync::Arc,
};

use qortoo::DatatypeHandler;

use crate::error::datatype_error_code;

/// State-change notification: `(userdata, old_state, new_state)` with `DatatypeState`
/// discriminants. Invoked on a Qortoo tokio worker thread.
pub type QortooOnStateChangeCallback =
    Option<extern "C" fn(userdata: usize, old_state: i32, new_state: i32)>;

/// Error notification: `(userdata, error_code, error_msg)`. `error_msg` is only valid
/// during the call. Invoked on a Qortoo tokio worker thread.
pub type QortooOnErrorCallback =
    Option<extern "C" fn(userdata: usize, code: i32, msg: *const c_char)>;

/// Called exactly once when the handler owning `userdata` is dropped, so the foreign
/// side can release resources tied to it (e.g., a Go `cgo.Handle`).
pub type QortooUserdataDropCallback = Option<extern "C" fn(userdata: usize)>;

/// Shared context for a foreign handler; drops the foreign userdata exactly once.
pub(crate) struct ForeignHandlerCtx {
    pub(crate) on_state_change: QortooOnStateChangeCallback,
    pub(crate) on_error: QortooOnErrorCallback,
    pub(crate) userdata: usize,
    pub(crate) userdata_drop: QortooUserdataDropCallback,
}

impl Drop for ForeignHandlerCtx {
    fn drop(&mut self) {
        if let Some(drop_fn) = self.userdata_drop {
            drop_fn(self.userdata);
        }
    }
}

pub(crate) fn make_foreign_handler(ctx: ForeignHandlerCtx) -> DatatypeHandler {
    let ctx = Arc::new(ctx);
    let mut handler = DatatypeHandler::new();
    if ctx.on_state_change.is_some() {
        let ctx = ctx.clone();
        handler = handler.set_on_state_change(move |_ds, old_state, new_state| {
            if let Some(f) = ctx.on_state_change {
                f(ctx.userdata, old_state as i32, new_state as i32);
            }
        });
    }
    if ctx.on_error.is_some() {
        let ctx = ctx.clone();
        handler = handler.set_on_error(move |_ds, err| {
            if let Some(f) = ctx.on_error {
                let code = datatype_error_code(&err);
                let msg = CString::new(err.to_string()).unwrap_or_default();
                f(ctx.userdata, code, msg.as_ptr());
            }
        });
    }
    handler
}
