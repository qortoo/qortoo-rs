//! End-to-end check that a foreign caller's W3C trace context reaches the core spans.
//!
//! A language binding is responsible for turning its context into the two headers; this
//! file covers everything after the headers arrive: the FFI span, the sync (resp.
//! transaction) that runs on the event-loop (resp. caller) thread, and the push/pull
//! spans the core emits there.
//!
//! `tracing`'s global subscriber can only be installed once per process, and the two
//! scenarios below both drive process-global tracing plus the shared background
//! event-loop runtime. Running them in the same process lets one scenario's spans
//! interleave with the other's ambient context, so — mirroring `observability.rs` —
//! each scenario runs in its own fresh copy of this test binary.

use std::{env, ffi::CString, process::Command, ptr, time::Duration};

use opentelemetry::trace::TracerProvider;
use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider};
use qortoo_ffi::{
    QortooCounter, QortooError, qortoo_client_free, qortoo_client_new, qortoo_counter_as_datatype,
    qortoo_counter_create, qortoo_counter_free, qortoo_counter_increase_by,
    qortoo_counter_transaction_with_context, qortoo_datatype_sync_with_context,
    qortoo_local_connectivity_free, qortoo_local_connectivity_new,
    qortoo_local_connectivity_set_realtime,
};
use tracing_subscriber::{EnvFilter, Registry, layer::SubscriberExt};

const TRACE_ID: &str = "4bf92f3577b34da6a3ce929d0e0e4736";
const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

const SUBPROCESS_ENV: &str = "QORTOO_FFI_TRACE_CONTEXT_TEST_SCENARIO";
const SYNC_SCENARIO: &str = "sync";
const TRANSACTION_SCENARIO: &str = "transaction";

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

fn run_isolated(scenario: &str) {
    let output = Command::new(env::current_exe().expect("resolve the current test binary"))
        .args(["--exact", "trace_context_subprocess", "--nocapture"])
        .env(SUBPROCESS_ENV, scenario)
        .output()
        .expect("run the trace-context test subprocess");

    assert!(
        output.status.success(),
        "trace-context scenario {scenario:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn can_continue_the_callers_trace_through_sync() {
    run_isolated(SYNC_SCENARIO);
}

#[test]
fn can_continue_the_callers_trace_through_transaction() {
    run_isolated(TRANSACTION_SCENARIO);
}

/// Entry point selected by `run_isolated`; a normal test run leaves it as a no-op.
#[test]
fn trace_context_subprocess() {
    let scenario = match env::var(SUBPROCESS_ENV) {
        Ok(scenario) => scenario,
        Err(env::VarError::NotPresent) => return,
        Err(error) => panic!("read {SUBPROCESS_ENV}: {error}"),
    };
    match scenario.as_str() {
        SYNC_SCENARIO => exercise_sync(),
        TRANSACTION_SCENARIO => exercise_transaction(),
        scenario => panic!("unknown trace-context subprocess scenario {scenario:?}"),
    }
}

fn exercise_sync() {
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

        qortoo_datatype_sync_with_context(
            qortoo_counter_as_datatype(counter),
            traceparent.as_ptr(),
            ptr::null(),
            &mut err,
        );
        assert_ok(&err, "sync with context");

        qortoo_counter_free(counter);
        qortoo_client_free(client);
        qortoo_local_connectivity_free(connectivity);
    }

    // `qortoo.sync` is the parent of `push_pull`, and a span's export via
    // `SimpleSpanProcessor` cannot complete until every child span it's tracking has
    // itself closed — so `push_pull` (which closes on the event-loop thread) can
    // appear in the exporter a moment before `qortoo.sync` does. Wait for both, not
    // just the child, or this poll can race the parent's own export.
    awaitility::at_most(Duration::from_secs(5))
        .poll_interval(Duration::from_millis(50))
        .until(|| {
            let spans = exporter.get_finished_spans().unwrap_or_default();
            spans.iter().any(|s| s.name == "push_pull")
                && spans.iter().any(|s| s.name == "qortoo.sync")
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

extern "C" fn commit_one_increase(tx_counter: *mut QortooCounter, _userdata: usize) -> i32 {
    let mut err = new_err();
    unsafe { qortoo_counter_increase_by(tx_counter, 1, &mut err) };
    assert_ok(&err, "increase_by inside transaction");
    0
}

fn exercise_transaction() {
    let exporter = install_recording_subscriber();

    let collection = CString::new("trace-context-e2e").unwrap();
    let alias = CString::new("client-b").unwrap();
    let key = CString::new("transaction-counter").unwrap();
    let tag = CString::new("trace-context-e2e-tx").unwrap();
    let traceparent = CString::new(TRACEPARENT).unwrap();
    let mut err = new_err();

    unsafe {
        let client = qortoo_client_new(collection.as_ptr(), alias.as_ptr(), ptr::null(), &mut err);
        assert_ok(&err, "client creation");

        let counter = qortoo_counter_create(client, key.as_ptr(), ptr::null(), &mut err);
        assert_ok(&err, "counter creation");

        qortoo_counter_transaction_with_context(
            counter,
            tag.as_ptr(),
            traceparent.as_ptr(),
            ptr::null(),
            commit_one_increase,
            0,
            &mut err,
        );
        assert_ok(&err, "transaction with context");

        qortoo_counter_free(counter);
        qortoo_client_free(client);
    }

    awaitility::at_most(Duration::from_secs(5))
        .poll_interval(Duration::from_millis(50))
        .until(|| {
            exporter
                .get_finished_spans()
                .unwrap_or_default()
                .iter()
                .any(|s| s.name == "qortoo.transaction")
        });

    let spans = exporter.get_finished_spans().unwrap();
    let tx_span = spans
        .iter()
        .find(|s| s.name == "qortoo.transaction")
        .expect("checked by the poll above");
    assert_eq!(
        tx_span.span_context.trace_id().to_string(),
        TRACE_ID,
        "the FFI transaction span must join the caller's trace"
    );
    assert!(
        tx_span.parent_span_id.to_string() == "00f067aa0ba902b7",
        "the FFI transaction span must hang off the caller's span, got {}",
        tx_span.parent_span_id
    );
}
