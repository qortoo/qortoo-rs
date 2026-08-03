//! Observability configuration, lifecycle, and process-global conflict behavior
//! exercised through the C ABI.
//!
//! Rust's tracing subscriber and metrics recorder cannot be reset. Scenarios that
//! install either global therefore run in fresh copies of this test binary, while
//! validation cases that never reach the lifecycle state machine run directly.

use std::{env, ffi::CString, process::Command, ptr};

use metrics_util::debugging::DebuggingRecorder;
use qortoo_ffi::{
    QORTOO_ERR_OBSERVABILITY_ALREADY_INITIALIZED, QORTOO_ERR_OBSERVABILITY_INVALID_CONFIG,
    QORTOO_ERR_OBSERVABILITY_PARTIALLY_INITIALIZED, QORTOO_ERR_OBSERVABILITY_RECORDER,
    QORTOO_ERR_OBSERVABILITY_SHUT_DOWN, QORTOO_ERR_OBSERVABILITY_SUBSCRIBER, QortooError,
    QortooObservabilityOptions, qortoo_observability_init, qortoo_observability_shutdown,
    qortoo_string_free,
};

const SUBPROCESS_ENV: &str = "QORTOO_FFI_OBSERVABILITY_TEST_SCENARIO";
const LIFECYCLE_SCENARIO: &str = "lifecycle";
const SUBSCRIBER_CONFLICT_SCENARIO: &str = "subscriber-conflict";
const RECORDER_CONFLICT_SCENARIO: &str = "recorder-conflict";

fn empty_options() -> QortooObservabilityOptions {
    QortooObservabilityOptions {
        service_name: ptr::null(),
        binding_language: ptr::null(),
        log_filter: ptr::null(),
        log_format: 0,
        trace_enabled: false,
        otlp_endpoint: ptr::null(),
        metrics_enabled: false,
        metrics_listen_addr: ptr::null(),
    }
}

fn new_err() -> QortooError {
    QortooError {
        code: 0,
        msg: ptr::null_mut(),
    }
}

/// Returns the error code and releases the message, mirroring what a binding does.
fn take_code(err: &mut QortooError) -> i32 {
    if !err.msg.is_null() {
        unsafe { qortoo_string_free(err.msg) };
        err.msg = ptr::null_mut();
    }
    let code = err.code;
    err.code = 0;
    code
}

fn run_isolated(scenario: &str) {
    let output = Command::new(env::current_exe().expect("resolve the current test binary"))
        .args(["--exact", "observability_subprocess", "--nocapture"])
        .env(SUBPROCESS_ENV, scenario)
        .output()
        .expect("run the observability test subprocess");

    assert!(
        output.status.success(),
        "observability scenario {scenario:?} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn can_initialize_once_and_shut_down_once() {
    run_isolated(LIFECYCLE_SCENARIO);
}

#[test]
fn can_report_a_subscriber_owned_by_the_application() {
    run_isolated(SUBSCRIBER_CONFLICT_SCENARIO);
}

#[test]
fn can_distinguish_a_retriable_recorder_conflict_from_a_partial_installation() {
    run_isolated(RECORDER_CONFLICT_SCENARIO);
}

/// Entry point selected by `run_isolated`; a normal test run leaves it as a no-op.
#[test]
fn observability_subprocess() {
    let scenario = match env::var(SUBPROCESS_ENV) {
        Ok(scenario) => scenario,
        Err(env::VarError::NotPresent) => return,
        Err(error) => panic!("read {SUBPROCESS_ENV}: {error}"),
    };
    match scenario.as_str() {
        LIFECYCLE_SCENARIO => exercise_lifecycle(),
        SUBSCRIBER_CONFLICT_SCENARIO => exercise_subscriber_conflict(),
        RECORDER_CONFLICT_SCENARIO => exercise_recorder_conflict(),
        scenario => panic!("unknown observability subprocess scenario {scenario:?}"),
    }
}

fn exercise_lifecycle() {
    let service = CString::new("qortoo-ffi-test").unwrap();
    let language = CString::new("go").unwrap();
    let filter = CString::new("qortoo=debug").unwrap();
    let mut options = empty_options();
    options.service_name = service.as_ptr();
    options.binding_language = language.as_ptr();
    options.log_filter = filter.as_ptr();
    // JSON logs only: no exporter, so the test needs neither a collector nor a port.
    options.log_format = 1;

    let mut err = new_err();
    unsafe { qortoo_observability_init(&options, &mut err) };
    assert_eq!(take_code(&mut err), 0, "the first init must succeed");

    unsafe { qortoo_observability_init(&options, &mut err) };
    assert_eq!(
        take_code(&mut err),
        QORTOO_ERR_OBSERVABILITY_ALREADY_INITIALIZED,
        "a second init must be reported, not silently ignored"
    );

    unsafe { qortoo_observability_shutdown(1_000, &mut err) };
    assert_eq!(take_code(&mut err), 0, "shutdown must succeed");

    unsafe { qortoo_observability_shutdown(1_000, &mut err) };
    assert_eq!(
        take_code(&mut err),
        0,
        "a repeated shutdown is a no-op, so deferred cleanup stays safe"
    );

    unsafe { qortoo_observability_init(&options, &mut err) };
    assert_eq!(
        take_code(&mut err),
        QORTOO_ERR_OBSERVABILITY_SHUT_DOWN,
        "the global subscriber cannot be reinstalled after shutdown"
    );
}

fn exercise_subscriber_conflict() {
    tracing::subscriber::set_global_default(tracing_subscriber::Registry::default())
        .expect("the subprocess owns a fresh process-global subscriber");

    let mut options = empty_options();
    options.log_format = 1;
    let mut err = new_err();

    unsafe { qortoo_observability_init(&options, &mut err) };
    assert_eq!(take_code(&mut err), QORTOO_ERR_OBSERVABILITY_SUBSCRIBER);

    // No Qortoo-owned global was installed, so the lifecycle remains retriable. The
    // unchanged external conflict is reported again rather than as a partial install.
    unsafe { qortoo_observability_init(&options, &mut err) };
    assert_eq!(take_code(&mut err), QORTOO_ERR_OBSERVABILITY_SUBSCRIBER);
}

fn exercise_recorder_conflict() {
    DebuggingRecorder::new()
        .install()
        .expect("the subprocess owns a fresh process-global recorder");

    // Port zero lets the OS select an unused listener while Prometheus is being built.
    let listen_addr = CString::new("127.0.0.1:0").unwrap();
    let mut options = empty_options();
    options.metrics_enabled = true;
    options.metrics_listen_addr = listen_addr.as_ptr();
    let mut err = new_err();

    unsafe { qortoo_observability_init(&options, &mut err) };
    assert_eq!(take_code(&mut err), QORTOO_ERR_OBSERVABILITY_RECORDER);

    // Metrics-only failure installed no Qortoo global, so a retry can still install the
    // subscriber. The same recorder conflict then makes that combined install partial.
    options.log_format = 1;
    unsafe { qortoo_observability_init(&options, &mut err) };
    assert_eq!(take_code(&mut err), QORTOO_ERR_OBSERVABILITY_RECORDER);

    unsafe { qortoo_observability_init(&options, &mut err) };
    assert_eq!(
        take_code(&mut err),
        QORTOO_ERR_OBSERVABILITY_PARTIALLY_INITIALIZED
    );
}

#[test]
fn can_reject_an_unknown_log_format() {
    let mut options = empty_options();
    options.log_format = 42;

    let mut err = new_err();
    unsafe { qortoo_observability_init(&options, &mut err) };
    assert_eq!(take_code(&mut err), QORTOO_ERR_OBSERVABILITY_INVALID_CONFIG);
}

#[test]
fn can_reject_an_invalid_metrics_address() {
    let addr = CString::new("definitely not an address").unwrap();
    let mut options = empty_options();
    options.metrics_enabled = true;
    options.metrics_listen_addr = addr.as_ptr();

    let mut err = new_err();
    unsafe { qortoo_observability_init(&options, &mut err) };
    assert_eq!(take_code(&mut err), QORTOO_ERR_OBSERVABILITY_INVALID_CONFIG);
}

#[test]
fn can_reject_an_invalid_log_filter() {
    let filter = CString::new("=nonsense=").unwrap();
    let mut options = empty_options();
    // A filter is only parsed when something consumes it, hence the stdout layer.
    options.log_format = 2;
    options.log_filter = filter.as_ptr();

    let mut err = new_err();
    unsafe { qortoo_observability_init(&options, &mut err) };
    assert_eq!(take_code(&mut err), QORTOO_ERR_OBSERVABILITY_INVALID_CONFIG);
}
