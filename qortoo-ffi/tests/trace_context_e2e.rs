//! End-to-end check that a foreign caller's W3C trace context reaches the core spans.
//!
//! The Go half of this path (turning a `context.Context` into the two headers) is
//! covered by `go/qortoo/observability_test.go`; this file covers everything after the
//! headers arrive: the FFI span, the sync that runs on the event-loop thread, and the
//! push/pull spans the core emits there.

use std::{ffi::CString, ptr, time::Duration};

use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use qortoo_ffi::{
    QortooError, qortoo_client_free, qortoo_client_new, qortoo_counter_create, qortoo_counter_free,
    qortoo_counter_increase_by, qortoo_counter_sync_with_context, qortoo_local_connectivity_free,
    qortoo_local_connectivity_new, qortoo_local_connectivity_set_realtime,
};
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};

const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

fn new_err() -> QortooError {
    QortooError {
        code: 0,
        msg: ptr::null_mut(),
    }
}

fn assert_ok(err: &QortooError, what: &str) {
    assert_eq!(err.code, 0, "{what} failed with code {}", err.code);
}

/// Installs a subscriber that records finished spans in memory, the way the OTLP
/// exporter would receive them.
fn install_recording_subscriber() -> InMemorySpanExporter {
    let exporter = InMemorySpanExporter::default();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .build();
    let subscriber = Registry::default()
        .with(EnvFilter::new("qortoo=trace,qortoo_ffi=trace"))
        .with(tracing_opentelemetry::layer().with_tracer(provider.tracer("test")));
    tracing::subscriber::set_global_default(subscriber).expect("no other global subscriber");
    exporter
}

#[test]
fn can_continue_the_callers_trace_through_sync() {
    let exporter = install_recording_subscriber();

    let collection = CString::new("trace-context-e2e").unwrap();
    let alias = CString::new("client-a").unwrap();
    let key = CString::new("counter").unwrap();
    let traceparent = CString::new(TRACEPARENT).unwrap();
    let mut err = new_err();

    unsafe {
        let connectivity = qortoo_local_connectivity_new();
        // Manual mode: the only sync in this test is the instrumented one below.
        qortoo_local_connectivity_set_realtime(connectivity, false);

        let client = qortoo_client_new(collection.as_ptr(), alias.as_ptr(), connectivity, &mut err);
        assert_ok(&err, "client creation");

        let counter = qortoo_counter_create(client, key.as_ptr(), ptr::null(), &mut err);
        assert_ok(&err, "counter creation");

        qortoo_counter_increase_by(counter, 7, &mut err);
        assert_ok(&err, "increase_by");

        qortoo_counter_sync_with_context(counter, traceparent.as_ptr(), ptr::null(), &mut err);
        assert_ok(&err, "sync with context");

        qortoo_counter_free(counter);
        qortoo_client_free(client);
        qortoo_local_connectivity_free(connectivity);
    }

    // The sync span closes on the caller thread, but push_pull closes on the event-loop
    // thread just before it answers, so give the exporter a moment on a slow machine.
    awaitility::at_most(Duration::from_secs(5))
        .poll_interval(Duration::from_millis(50))
        .until(|| {
            exporter
                .get_finished_spans()
                .unwrap_or_default()
                .iter()
                .any(|s| s.name == "push_pull")
        });

    let spans = exporter.get_finished_spans().unwrap();
    let named = |name: &str| spans.iter().find(|s| s.name == name);

    let sync_span = named("qortoo.sync").expect("the FFI opens a span around sync");
    assert_eq!(
        sync_span.span_context.trace_id().to_string(),
        TRACE_ID,
        "the FFI span must join the caller's trace"
    );
    assert!(
        sync_span.parent_span_id.to_string() == "00f067aa0ba902b7",
        "the FFI span must hang off the caller's span, got {}",
        sync_span.parent_span_id
    );

    let push_pull = named("push_pull").expect("checked by the poll above");
    assert_eq!(
        push_pull.span_context.trace_id().to_string(),
        TRACE_ID,
        "the sync that runs on the event-loop thread must stay in the caller's trace"
    );
}
