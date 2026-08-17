//! Handler callback forwarding and userdata-ownership contracts, exercised through the
//! C ABI (`QortooDatatypeOptions`, `qortoo_counter_set_handler`,
//! `qortoo_counter_unset_handler`).
//!
//! Every scenario records into a registry keyed by a unique `userdata` value (from
//! `support::unique_userdata`), so parallel tests sharing this binary never observe
//! each other's callbacks.

mod support;

use std::{
    collections::HashMap,
    ffi::{CStr, c_char},
    ptr,
    sync::{Mutex, OnceLock},
    time::Duration,
};

use qortoo_ffi::{
    QortooDatatypeOptions, qortoo_client_new, qortoo_counter_create, qortoo_counter_increase,
    qortoo_counter_set_handler, qortoo_counter_sync, qortoo_counter_unset_handler,
    qortoo_local_connectivity_new, qortoo_local_connectivity_set_realtime,
};
use support::*;

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Event {
    StateChange { old: i32, new: i32 },
    Error { code: i32, msg: String },
    Drop,
}

fn registry() -> &'static Mutex<HashMap<usize, Vec<Event>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<usize, Vec<Event>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn record(userdata: usize, event: Event) {
    registry()
        .lock()
        .unwrap()
        .entry(userdata)
        .or_default()
        .push(event);
}

fn events_for(userdata: usize) -> Vec<Event> {
    registry()
        .lock()
        .unwrap()
        .get(&userdata)
        .cloned()
        .unwrap_or_default()
}

fn await_event(userdata: usize, matches: impl Fn(&Event) -> bool + 'static) {
    awaitility::at_most(Duration::from_secs(5))
        .poll_interval(Duration::from_millis(10))
        .until(|| events_for(userdata).iter().any(&matches));
}

extern "C" fn on_state_change_cb(userdata: usize, old_state: i32, new_state: i32) {
    record(
        userdata,
        Event::StateChange {
            old: old_state,
            new: new_state,
        },
    );
}

extern "C" fn on_error_cb(userdata: usize, code: i32, msg: *const c_char) {
    let msg = if msg.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(msg) }
            .to_string_lossy()
            .into_owned()
    };
    record(userdata, Event::Error { code, msg });
}

extern "C" fn on_drop_cb(userdata: usize) {
    record(userdata, Event::Drop);
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn new_client(conn: *const qortoo_ffi::QortooLocalConnectivity) -> ClientGuard {
    let collection = unique_cstring("collection");
    let alias = unique_cstring("alias");
    let mut err = new_err();
    let client = unsafe { qortoo_client_new(collection.as_ptr(), alias.as_ptr(), conn, &mut err) };
    assert_ok(&mut err, "client creation");
    ClientGuard(client)
}

fn manual_conn() -> ConnGuard {
    let conn = qortoo_local_connectivity_new();
    unsafe { qortoo_local_connectivity_set_realtime(conn, false) };
    ConnGuard(conn)
}

fn empty_options() -> QortooDatatypeOptions {
    QortooDatatypeOptions {
        readonly: false,
        max_push_buffer_size: 0,
        handler_priority: 0,
        on_state_change: None,
        on_error: None,
        handler_userdata: 0,
        handler_userdata_drop: None,
    }
}

// ---------------------------------------------------------------------------
// State-change forwarding
// ---------------------------------------------------------------------------

#[test]
fn can_receive_a_state_transition_via_an_options_registered_handler() {
    let conn = manual_conn();
    let client = new_client(conn.0);
    let userdata = unique_userdata();
    let mut options = empty_options();
    options.on_state_change = Some(on_state_change_cb);
    options.handler_userdata = userdata;

    let mut err = new_err();
    let counter = unsafe {
        qortoo_counter_create(client.0, unique_cstring("key").as_ptr(), &options, &mut err)
    };
    assert_ok(&mut err, "create with a state-change handler");
    let counter = CounterGuard(counter);

    unsafe { qortoo_counter_sync(counter.0, &mut err) };
    assert_ok(&mut err, "sync (Creating -> Subscribed)");

    await_event(userdata, |e| {
        *e == Event::StateChange { old: 0, new: 3 } // Creating -> Subscribed
    });
}

#[test]
fn can_receive_a_state_transition_via_set_handler() {
    let conn = manual_conn();
    let client = new_client(conn.0);
    let mut err = new_err();
    let counter = unsafe {
        qortoo_counter_create(
            client.0,
            unique_cstring("key").as_ptr(),
            ptr::null(),
            &mut err,
        )
    };
    assert_ok(&mut err, "create without a handler");
    let counter = CounterGuard(counter);

    let userdata = unique_userdata();
    unsafe {
        qortoo_counter_set_handler(counter.0, 0, Some(on_state_change_cb), None, userdata, None)
    };

    unsafe { qortoo_counter_sync(counter.0, &mut err) };
    assert_ok(&mut err, "sync (Creating -> Subscribed)");

    await_event(userdata, |e| *e == Event::StateChange { old: 0, new: 3 });
}

// ---------------------------------------------------------------------------
// Error forwarding
// ---------------------------------------------------------------------------

#[test]
fn can_forward_a_datatype_error_with_its_mapped_code_and_message() {
    // Same buffer-exceeded trigger as `client_counter.rs`'s push-buffer test: the
    // error only ever reaches the caller through the registered `on_error` handler,
    // never through `qortoo_counter_increase`'s synchronous `err_out`.
    let conn = manual_conn();
    let client = new_client(conn.0);
    let userdata = unique_userdata();
    let mut options = empty_options();
    options.max_push_buffer_size = 1; // clamped up to the SDK's 1MB floor
    options.on_error = Some(on_error_cb);
    options.handler_userdata = userdata;

    let mut err = new_err();
    let counter = unsafe {
        qortoo_counter_create(client.0, unique_cstring("key").as_ptr(), &options, &mut err)
    };
    assert_ok(
        &mut err,
        "create with a bounded push buffer and an error handler",
    );
    let counter = CounterGuard(counter);

    const SAFETY_BOUND: u32 = 50_000;
    for _ in 0..SAFETY_BOUND {
        unsafe { qortoo_counter_increase(counter.0, &mut err) };
        take_code(&mut err);
        if events_for(userdata)
            .iter()
            .any(|e| matches!(e, Event::Error { .. }))
        {
            break;
        }
    }

    await_event(userdata, |e| matches!(e, Event::Error { .. }));
    let events = events_for(userdata);
    let Event::Error { code, msg } = events
        .iter()
        .find(|e| matches!(e, Event::Error { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(*code, 211 /* PushBufferExceededMaxMemSize */);
    assert!(
        !msg.is_empty(),
        "the callback-scoped message must be non-empty"
    );
}

// ---------------------------------------------------------------------------
// Replace / unset
// ---------------------------------------------------------------------------

#[test]
fn can_release_the_old_userdata_exactly_once_when_replacing_a_handler() {
    let client = new_client(ptr::null());
    let mut err = new_err();
    let counter = unsafe {
        qortoo_counter_create(
            client.0,
            unique_cstring("key").as_ptr(),
            ptr::null(),
            &mut err,
        )
    };
    assert_ok(&mut err, "create");
    let counter = CounterGuard(counter);

    // `on_state_change` must be non-null here: `make_foreign_handler` only captures
    // the userdata context inside a closure it actually registers, so a handler with
    // both callbacks null releases its userdata immediately (see
    // `can_release_userdata_immediately_when_no_callback_is_supplied`) rather than staying
    // alive until it is replaced — which would make this test observe the release for
    // the wrong reason.
    let old_userdata = unique_userdata();
    let new_userdata = unique_userdata();
    unsafe {
        qortoo_counter_set_handler(
            counter.0,
            0,
            Some(on_state_change_cb),
            None,
            old_userdata,
            Some(on_drop_cb),
        )
    };
    unsafe {
        qortoo_counter_set_handler(
            counter.0,
            0,
            Some(on_state_change_cb),
            None,
            new_userdata,
            Some(on_drop_cb),
        )
    };

    await_event(old_userdata, |e| *e == Event::Drop);
    assert_eq!(
        events_for(old_userdata)
            .iter()
            .filter(|e| **e == Event::Drop)
            .count(),
        1,
        "the replaced handler's userdata must be released exactly once"
    );
    assert!(
        events_for(new_userdata).is_empty(),
        "the replacing handler's userdata must still be live"
    );
}

#[test]
fn can_unset_a_handler_and_release_its_userdata_exactly_once() {
    let client = new_client(ptr::null());
    let mut err = new_err();
    let counter = unsafe {
        qortoo_counter_create(
            client.0,
            unique_cstring("key").as_ptr(),
            ptr::null(),
            &mut err,
        )
    };
    assert_ok(&mut err, "create");
    let counter = CounterGuard(counter);

    // See the comment in `can_release_the_old_userdata_exactly_once_when_replacing_a_handler`:
    // a non-null callback is required so the userdata survives until this `unset` call.
    let userdata = unique_userdata();
    unsafe {
        qortoo_counter_set_handler(
            counter.0,
            3,
            Some(on_state_change_cb),
            None,
            userdata,
            Some(on_drop_cb),
        )
    };

    let removed = unsafe { qortoo_counter_unset_handler(counter.0, 3) };
    assert!(removed, "unsetting a registered priority must return true");
    await_event(userdata, |e| *e == Event::Drop);
    assert_eq!(
        events_for(userdata)
            .iter()
            .filter(|e| **e == Event::Drop)
            .count(),
        1
    );

    let removed_again = unsafe { qortoo_counter_unset_handler(counter.0, 3) };
    assert!(
        !removed_again,
        "unsetting an already-removed priority must return false"
    );
    assert_eq!(
        events_for(userdata)
            .iter()
            .filter(|e| **e == Event::Drop)
            .count(),
        1,
        "unsetting a missing handler must not trigger an extra destruction"
    );
}

// ---------------------------------------------------------------------------
// Userdata ownership at construction time
// ---------------------------------------------------------------------------

#[test]
fn can_release_userdata_immediately_when_no_callback_is_supplied() {
    let client = new_client(ptr::null());
    let userdata = unique_userdata();
    let mut options = empty_options();
    // Both callbacks absent: Rust retains no handler, so the FFI layer must release
    // the userdata immediately rather than leak it silently.
    options.handler_userdata = userdata;
    options.handler_userdata_drop = Some(on_drop_cb);

    let mut err = new_err();
    let counter = unsafe {
        qortoo_counter_create(client.0, unique_cstring("key").as_ptr(), &options, &mut err)
    };
    assert_ok(&mut err, "create with a callback-less handler option");
    let _counter = CounterGuard(counter);

    assert_eq!(events_for(userdata), vec![Event::Drop]);
}

#[test]
fn can_release_userdata_after_an_early_null_client_construction_failure() {
    let userdata = unique_userdata();
    let mut options = empty_options();
    options.on_state_change = Some(on_state_change_cb);
    options.handler_userdata = userdata;
    options.handler_userdata_drop = Some(on_drop_cb);

    let mut err = new_err();
    let counter = unsafe {
        qortoo_counter_create(
            ptr::null(),
            unique_cstring("key").as_ptr(),
            &options,
            &mut err,
        )
    };
    assert!(counter.is_null());
    assert_err(
        &mut err,
        qortoo_ffi::QORTOO_ERR_INVALID_ARGUMENT,
        "null client",
    );
    assert_eq!(events_for(userdata), vec![Event::Drop]);
}

#[test]
fn can_release_userdata_after_a_late_duplicate_key_construction_failure() {
    let client = new_client(ptr::null());
    let key = unique_cstring("key");
    let mut err = new_err();
    let first = unsafe { qortoo_counter_create(client.0, key.as_ptr(), ptr::null(), &mut err) };
    assert_ok(&mut err, "first construction");
    let _first = CounterGuard(first);

    let userdata = unique_userdata();
    let mut options = empty_options();
    options.on_state_change = Some(on_state_change_cb);
    options.handler_userdata = userdata;
    options.handler_userdata_drop = Some(on_drop_cb);

    let second = unsafe { qortoo_counter_create(client.0, key.as_ptr(), &options, &mut err) };
    assert!(second.is_null());
    assert_err(&mut err, 101, "duplicate key construction");
    assert_eq!(events_for(userdata), vec![Event::Drop]);
}

#[test]
fn can_release_userdata_immediately_when_setting_a_handler_on_a_null_counter() {
    let userdata = unique_userdata();
    unsafe { qortoo_counter_set_handler(ptr::null(), 0, None, None, userdata, Some(on_drop_cb)) };
    assert_eq!(events_for(userdata), vec![Event::Drop]);
}

// ---------------------------------------------------------------------------
// Null safety
// ---------------------------------------------------------------------------

#[test]
fn can_accept_null_callbacks_and_userdata_drop_safely() {
    let client = new_client(ptr::null());
    let mut err = new_err();
    let counter = unsafe {
        qortoo_counter_create(
            client.0,
            unique_cstring("key").as_ptr(),
            ptr::null(),
            &mut err,
        )
    };
    assert_ok(&mut err, "create");
    let counter = CounterGuard(counter);

    // Fully null handler on a live counter: must not crash.
    unsafe { qortoo_counter_set_handler(counter.0, 0, None, None, 0, None) };
    // Fully null handler on a null counter: must not crash.
    unsafe { qortoo_counter_set_handler(ptr::null(), 0, None, None, 0, None) };
}
