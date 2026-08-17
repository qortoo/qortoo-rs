//! W3C trace-context bridge: turns the `traceparent`/`tracestate` headers of a foreign
//! caller into the parent of the span the FFI opens around the call.
//!
//! An OpenTelemetry context does not cross the cgo boundary on its own, so a Go caller
//! passes the two header values explicitly. Without them — or without an installed
//! `tracing_opentelemetry` layer — everything here degrades to a plain local span.

use std::{collections::HashMap, ffi::CStr, os::raw::c_char};

use opentelemetry::{
    Context,
    propagation::{Extractor, TextMapPropagator},
    trace::TraceContextExt,
};
use opentelemetry_sdk::propagation::TraceContextPropagator;
use tracing::Span;
use tracing_opentelemetry::OpenTelemetrySpanExt;

const TRACEPARENT: &str = "traceparent";
const TRACESTATE: &str = "tracestate";

/// Opens a span with the given static name, parented to the remote context carried by
/// `traceparent`/`tracestate` (both nullable), and runs `body` inside it.
///
/// Must be invoked from an `unsafe` context: the two header pointers are dereferenced.
macro_rules! with_remote_parent {
    ($name:literal, $traceparent:expr, $tracestate:expr, $body:expr) => {{
        let span = tracing::info_span!($name);
        $crate::observability::trace_context::set_remote_parent(&span, $traceparent, $tracestate);
        span.in_scope($body)
    }};
}

pub(crate) use with_remote_parent;

/// Sets the remote parent of `span` from the two C strings, if they carry a valid
/// context. Invalid or absent headers leave the span with its local parent.
///
/// # Safety
///
/// Both pointers must be null or NUL-terminated strings valid for the call.
pub(crate) unsafe fn set_remote_parent(
    span: &Span,
    traceparent: *const c_char,
    tracestate: *const c_char,
) {
    let traceparent = unsafe { optional_str(traceparent) };
    let tracestate = unsafe { optional_str(tracestate) };
    if let Some(context) = extract_context(traceparent.as_deref(), tracestate.as_deref()) {
        // Fails when no `tracing_opentelemetry` layer is installed, i.e. when the
        // application never called `qortoo_observability_init` — the intended no-op.
        let _ = span.set_parent(context);
    }
}

/// Returns the remote context, or `None` when the headers do not describe a valid span.
fn extract_context(traceparent: Option<&str>, tracestate: Option<&str>) -> Option<Context> {
    let mut carrier = HashMap::new();
    carrier.insert(TRACEPARENT.to_string(), traceparent?.to_string());
    if let Some(state) = tracestate {
        carrier.insert(TRACESTATE.to_string(), state.to_string());
    }
    let context = TraceContextPropagator::new().extract(&Carrier(carrier));
    context.span().span_context().is_valid().then_some(context)
}

/// `HashMap` wrapper implementing the OpenTelemetry [`Extractor`] contract.
struct Carrier(HashMap<String, String>);

impl Extractor for Carrier {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(String::as_str).collect()
    }
}

/// Reads a nullable C string, treating invalid UTF-8 as absent.
unsafe fn optional_str(p: *const c_char) -> Option<String> {
    if p.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(p) }
        .to_str()
        .ok()
        .map(str::to_owned)
}

#[cfg(test)]
mod tests_trace_context {
    use opentelemetry::trace::TraceContextExt;

    use super::*;

    const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
    const SPAN_ID: &str = "00f067aa0ba902b7";

    fn valid_traceparent() -> String {
        format!("00-{TRACE_ID}-{SPAN_ID}-01")
    }

    #[test]
    fn can_extract_a_valid_remote_context() {
        let context = extract_context(Some(&valid_traceparent()), None).expect("valid header");
        let span_context = context.span().span_context().clone();

        assert_eq!(span_context.trace_id().to_string(), TRACE_ID);
        assert_eq!(span_context.span_id().to_string(), SPAN_ID);
        assert!(span_context.is_remote());
        assert!(span_context.is_sampled());
    }

    #[test]
    fn can_keep_the_tracestate_of_the_caller() {
        let context = extract_context(Some(&valid_traceparent()), Some("vendor=value"))
            .expect("valid header");

        assert_eq!(
            context.span().span_context().trace_state().get("vendor"),
            Some("value")
        );
    }

    #[test]
    fn can_fall_back_to_no_parent_on_missing_or_broken_headers() {
        assert!(extract_context(None, None).is_none());
        assert!(extract_context(Some(""), None).is_none());
        assert!(extract_context(Some("garbage"), None).is_none());
        // Well-formed shape, but an all-zero (invalid) trace id.
        assert!(
            extract_context(
                Some("00-00000000000000000000000000000000-0000000000000000-01"),
                None
            )
            .is_none()
        );
    }

    #[test]
    fn can_read_nullable_c_strings() {
        let text = std::ffi::CString::new("value").unwrap();
        assert_eq!(
            unsafe { optional_str(text.as_ptr()) },
            Some("value".to_string())
        );
        assert_eq!(unsafe { optional_str(std::ptr::null()) }, None);
    }

    #[test]
    fn can_treat_invalid_utf8_c_strings_as_absent() {
        // A stray UTF-8 continuation byte, NUL-terminated.
        let bytes: [c_char; 3] = ['b' as c_char, 0x80u8 as c_char, 0];
        assert_eq!(unsafe { optional_str(bytes.as_ptr()) }, None);
    }

    #[test]
    fn can_expose_every_inserted_key_via_carrier_keys() {
        let mut map = HashMap::new();
        map.insert(TRACEPARENT.to_string(), valid_traceparent());
        map.insert(TRACESTATE.to_string(), "vendor=value".to_string());
        let carrier = Carrier(map);

        let mut keys = carrier.keys();
        keys.sort_unstable();
        assert_eq!(keys, vec![TRACEPARENT, TRACESTATE]);
    }
}
