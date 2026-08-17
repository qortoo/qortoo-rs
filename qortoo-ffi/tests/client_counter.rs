//! `Client` and `Counter` lifecycle contracts exercised through the C ABI: construction,
//! pointer/UTF-8 validation, ownership, and the exact error-code mapping a binding
//! relies on.
//!
//! Deliberate use-after-free, double-free, and non-NUL-terminated input are out of
//! scope: those invoke undefined behavior rather than a supported contract.

mod support;

use std::ptr;

use qortoo_ffi::{
    QORTOO_ERR_INVALID_ARGUMENT, QortooDatatypeOptions, qortoo_client_get_alias,
    qortoo_client_get_collection, qortoo_client_new, qortoo_client_unsubscribe_datatype,
    qortoo_counter_create, qortoo_counter_free, qortoo_counter_get_client_version,
    qortoo_counter_get_key, qortoo_counter_get_server_version, qortoo_counter_get_state,
    qortoo_counter_get_synced_client_version, qortoo_counter_get_type, qortoo_counter_get_value,
    qortoo_counter_increase, qortoo_counter_increase_by, qortoo_counter_subscribe,
    qortoo_counter_subscribe_or_create, qortoo_counter_sync, qortoo_counter_sync_with_context,
    qortoo_counter_unset_handler, qortoo_counter_unsubscribe, qortoo_local_connectivity_new,
    qortoo_local_connectivity_set_realtime,
};
use rstest::rstest;
use support::*;

/// A pointer-argument corruption shared by several `#[rstest]` groups below: every
/// string argument across this crate's FFI surface goes through the same `cstr_arg`
/// gate, so "null" and "invalid UTF-8" are the two cases worth repeating per argument.
#[derive(Clone, Copy)]
enum BadArg {
    Null,
    InvalidUtf8,
}

fn null_options() -> *const QortooDatatypeOptions {
    ptr::null()
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

/// A manual-mode connectivity backend, guarded for release.
fn manual_conn() -> ConnGuard {
    let conn = qortoo_local_connectivity_new();
    unsafe { qortoo_local_connectivity_set_realtime(conn, false) };
    ConnGuard(conn)
}

/// Builds a client on the given connectivity (null selects the no-op backend), with a
/// unique collection and alias so parallel tests never collide.
fn new_client(conn: *const qortoo_ffi::QortooLocalConnectivity) -> ClientGuard {
    let collection = unique_cstring("collection");
    let alias = unique_cstring("alias");
    let mut err = new_err();
    let client = unsafe { qortoo_client_new(collection.as_ptr(), alias.as_ptr(), conn, &mut err) };
    assert_ok(&mut err, "client creation");
    assert!(!client.is_null());
    ClientGuard(client)
}

// ---------------------------------------------------------------------------
// Client construction
// ---------------------------------------------------------------------------

// Client construction with both null and real connectivity is already exercised as
// setup by most other tests in this file (`new_client`/`manual_conn`); a dedicated
// smoke test would only re-prove what a failure there would already surface loudly
// elsewhere. `can_read_back_the_collection_and_alias` below is the one dedicated
// construction test worth keeping, since it makes an assertion nothing else does.

#[test]
fn can_read_back_the_collection_and_alias() {
    let collection = unique_cstring("collection");
    let alias = unique_cstring("alias");
    let mut err = new_err();
    let client =
        unsafe { qortoo_client_new(collection.as_ptr(), alias.as_ptr(), ptr::null(), &mut err) };
    assert_ok(&mut err, "client creation");
    let client = ClientGuard(client);

    let got_collection = read_and_free_string(unsafe { qortoo_client_get_collection(client.0) });
    let got_alias = read_and_free_string(unsafe { qortoo_client_get_alias(client.0) });

    assert_eq!(got_collection.as_deref(), collection.to_str().ok());
    assert_eq!(got_alias.as_deref(), alias.to_str().ok());
}

/// Which of `qortoo_client_new`'s two string arguments a case corrupts.
#[derive(Clone, Copy)]
enum ClientNewField {
    Collection,
    Alias,
}

#[rstest]
#[case::null_collection(ClientNewField::Collection, BadArg::Null)]
#[case::null_alias(ClientNewField::Alias, BadArg::Null)]
#[case::invalid_utf8_collection(ClientNewField::Collection, BadArg::InvalidUtf8)]
#[case::invalid_utf8_alias(ClientNewField::Alias, BadArg::InvalidUtf8)]
fn can_reject_a_bad_client_new_argument(#[case] field: ClientNewField, #[case] bad: BadArg) {
    let good_collection = unique_cstring("collection");
    let good_alias = unique_cstring("alias");
    let invalid = InvalidUtf8CString::new();
    let bad_ptr = match bad {
        BadArg::Null => ptr::null(),
        BadArg::InvalidUtf8 => invalid.as_ptr(),
    };
    let (collection_ptr, alias_ptr) = match field {
        ClientNewField::Collection => (bad_ptr, good_alias.as_ptr()),
        ClientNewField::Alias => (good_collection.as_ptr(), bad_ptr),
    };

    let mut err = new_err();
    let client = unsafe { qortoo_client_new(collection_ptr, alias_ptr, ptr::null(), &mut err) };
    assert!(client.is_null());
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "bad client_new argument",
    );
}

#[test]
fn can_map_an_invalid_collection_name_to_code_100() {
    // Collection names cannot be empty (see `is_valid_collection_name`).
    let collection = std::ffi::CString::new("").unwrap();
    let alias = unique_cstring("alias");
    let mut err = new_err();
    let client =
        unsafe { qortoo_client_new(collection.as_ptr(), alias.as_ptr(), ptr::null(), &mut err) };
    assert!(client.is_null());
    assert_err(&mut err, 100, "empty collection name");
}

#[test]
fn can_free_a_null_client_safely() {
    unsafe { qortoo_ffi::qortoo_client_free(ptr::null_mut()) };
}

#[test]
fn can_use_null_safe_client_getters() {
    assert!(unsafe { qortoo_client_get_collection(ptr::null()) }.is_null());
    assert!(unsafe { qortoo_client_get_alias(ptr::null()) }.is_null());
}

#[derive(Clone, Copy)]
enum BadUnsubscribeArg {
    NullClient,
    InvalidUtf8Key,
}

#[rstest]
#[case::null_client(BadUnsubscribeArg::NullClient)]
#[case::invalid_utf8_key(BadUnsubscribeArg::InvalidUtf8Key)]
fn can_reject_a_bad_unsubscribe_datatype_argument(#[case] bad: BadUnsubscribeArg) {
    let client = new_client(ptr::null());
    let good_key = unique_cstring("key");
    let invalid = InvalidUtf8CString::new();
    let (client_ptr, key_ptr) = match bad {
        BadUnsubscribeArg::NullClient => (ptr::null(), good_key.as_ptr()),
        BadUnsubscribeArg::InvalidUtf8Key => (client.0 as *const _, invalid.as_ptr()),
    };

    let mut err = new_err();
    unsafe { qortoo_client_unsubscribe_datatype(client_ptr, key_ptr, &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "bad unsubscribe_datatype argument",
    );
}

// The client-driven unsubscribe path (and its `Unsubscribing` transition) is covered
// by `can_transition_state_through_both_unsubscribe_paths` below, alongside the
// counter-driven path — a standalone test here would only repeat half of it.

// ---------------------------------------------------------------------------
// Counter construction
// ---------------------------------------------------------------------------

#[test]
fn can_construct_counters_with_the_expected_initial_state_per_mode() {
    let client = new_client(ptr::null());
    let mut err = new_err();

    let created = unsafe {
        qortoo_counter_create(
            client.0,
            unique_cstring("create").as_ptr(),
            null_options(),
            &mut err,
        )
    };
    assert_ok(&mut err, "create");
    let created = CounterGuard(created);
    assert_eq!(
        unsafe { qortoo_counter_get_state(created.0) },
        0 /* Creating */
    );

    let subscribed = unsafe {
        qortoo_counter_subscribe(
            client.0,
            unique_cstring("subscribe").as_ptr(),
            null_options(),
            &mut err,
        )
    };
    assert_ok(&mut err, "subscribe");
    let subscribed = CounterGuard(subscribed);
    assert_eq!(
        unsafe { qortoo_counter_get_state(subscribed.0) },
        1 /* Subscribing */
    );

    let sub_or_create = unsafe {
        qortoo_counter_subscribe_or_create(
            client.0,
            unique_cstring("subscribe-or-create").as_ptr(),
            null_options(),
            &mut err,
        )
    };
    assert_ok(&mut err, "subscribe_or_create");
    let sub_or_create = CounterGuard(sub_or_create);
    assert_eq!(
        unsafe { qortoo_counter_get_state(sub_or_create.0) },
        2 /* SubscribingOrCreating */
    );
}

// `qortoo_counter_create` with null options is exercised as setup by nearly every
// other test in this file. A populated-but-callback-less `QortooDatatypeOptions` is
// exercised more rigorously by `handler.rs`'s
// `can_release_userdata_immediately_when_no_callback_is_supplied`, which proves
// construction succeeds *and* that the userdata drop fires — a strictly stronger
// version of what a standalone construction-only test here would show.

#[test]
fn can_reject_a_write_on_a_readonly_counter_with_code_207() {
    let client = new_client(ptr::null());
    let mut options = empty_options();
    options.readonly = true;
    let mut err = new_err();
    // `Creating` is a writable state, so this isolates the readonly flag rather than
    // the state-based `NotWritable` gate a `Subscribing`/`SubscribingOrCreating`
    // counter would also hit before ever consulting the readonly flag.
    let counter = unsafe {
        qortoo_counter_create(client.0, unique_cstring("key").as_ptr(), &options, &mut err)
    };
    assert_ok(&mut err, "create readonly counter");
    let counter = CounterGuard(counter);

    let value = unsafe { qortoo_counter_increase(counter.0, &mut err) };
    assert_eq!(value, 0);
    assert_err(
        &mut err,
        207, /* ReadonlyViolation */
        "write to readonly counter",
    );
}

// `max_push_buffer_size`'s wiring to `PushBufferExceededMaxMemSize` (code 211),
// delivered asynchronously through `on_error`, is covered once in `handler.rs`'s
// `can_forward_a_datatype_error_with_its_mapped_code_and_message` — the same trigger
// re-run here would only re-verify core push-buffer accounting, not anything specific
// to this file's client/counter construction contracts.

#[test]
fn can_map_duplicate_construction_to_client_error_code_101() {
    let client = new_client(ptr::null());
    let key = unique_cstring("key");
    let mut err = new_err();
    let first = unsafe { qortoo_counter_create(client.0, key.as_ptr(), null_options(), &mut err) };
    assert_ok(&mut err, "first construction");
    let _first = CounterGuard(first);

    let second = unsafe { qortoo_counter_create(client.0, key.as_ptr(), null_options(), &mut err) };
    assert!(second.is_null());
    assert_err(&mut err, 101, "duplicate key construction");
}

#[derive(Clone, Copy)]
enum BadCreateArg {
    NullClient,
    NullKey,
    InvalidUtf8Key,
    /// Keys starting with `$` are rejected by `is_valid_datatype_key` — a synchronous
    /// core validation error, not a boundary `cstr_arg` rejection, hence its own code.
    InvalidKeySyntax,
}

#[rstest]
#[case::null_client(BadCreateArg::NullClient, QORTOO_ERR_INVALID_ARGUMENT)]
#[case::null_key(BadCreateArg::NullKey, QORTOO_ERR_INVALID_ARGUMENT)]
#[case::invalid_utf8_key(BadCreateArg::InvalidUtf8Key, QORTOO_ERR_INVALID_ARGUMENT)]
#[case::invalid_key_syntax(BadCreateArg::InvalidKeySyntax, 101)]
fn can_reject_a_bad_counter_create_argument(#[case] bad: BadCreateArg, #[case] expected_code: i32) {
    let client = new_client(ptr::null());
    let good_key = unique_cstring("key");
    let invalid = InvalidUtf8CString::new();
    let reserved_key = std::ffi::CString::new("$reserved").unwrap();
    let (client_ptr, key_ptr) = match bad {
        BadCreateArg::NullClient => (ptr::null(), good_key.as_ptr()),
        BadCreateArg::NullKey => (client.0 as *const _, ptr::null()),
        BadCreateArg::InvalidUtf8Key => (client.0 as *const _, invalid.as_ptr()),
        BadCreateArg::InvalidKeySyntax => (client.0 as *const _, reserved_key.as_ptr()),
    };

    let mut err = new_err();
    let counter = unsafe { qortoo_counter_create(client_ptr, key_ptr, null_options(), &mut err) };
    assert!(counter.is_null());
    assert_err(&mut err, expected_code, "bad counter_create argument");
}

static USERDATA_DROP_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

extern "C" fn count_userdata_drop(_userdata: usize) {
    USERDATA_DROP_COUNT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
}

extern "C" fn noop_on_state_change(_userdata: usize, _old: i32, _new: i32) {}

#[test]
fn can_release_handler_userdata_exactly_once_on_construction_failure() {
    // Detailed callback-forwarding assertions live in `handler.rs`; this test only
    // proves the exactly-once release on a *construction* failure path.
    let client = new_client(ptr::null());
    let key = unique_cstring("key");
    let mut err = new_err();
    let first = unsafe { qortoo_counter_create(client.0, key.as_ptr(), null_options(), &mut err) };
    assert_ok(&mut err, "first construction");
    let _first = CounterGuard(first);

    let before = USERDATA_DROP_COUNT.load(std::sync::atomic::Ordering::SeqCst);
    let mut options = empty_options();
    options.on_state_change = Some(noop_on_state_change);
    options.handler_userdata_drop = Some(count_userdata_drop);

    let second = unsafe { qortoo_counter_create(client.0, key.as_ptr(), &options, &mut err) };
    assert!(second.is_null());
    assert_err(&mut err, 101, "duplicate key construction");

    assert_eq!(
        USERDATA_DROP_COUNT.load(std::sync::atomic::Ordering::SeqCst) - before,
        1,
        "userdata must be released exactly once on a construction failure"
    );
}

// ---------------------------------------------------------------------------
// Counter operations
// ---------------------------------------------------------------------------

#[test]
fn can_increase_and_read_back_the_counter_value() {
    let client = new_client(ptr::null());
    let mut err = new_err();
    let counter = unsafe {
        qortoo_counter_create(
            client.0,
            unique_cstring("key").as_ptr(),
            null_options(),
            &mut err,
        )
    };
    assert_ok(&mut err, "create");
    let counter = CounterGuard(counter);

    assert_eq!(unsafe { qortoo_counter_increase(counter.0, &mut err) }, 1);
    assert_ok(&mut err, "increase");
    assert_eq!(
        unsafe { qortoo_counter_increase_by(counter.0, 5, &mut err) },
        6
    );
    assert_ok(&mut err, "increase_by positive");
    assert_eq!(
        unsafe { qortoo_counter_increase_by(counter.0, -2, &mut err) },
        4
    );
    assert_ok(&mut err, "increase_by negative");
    assert_eq!(unsafe { qortoo_counter_get_value(counter.0) }, 4);
}

#[test]
fn can_report_key_type_and_versions_via_the_getters() {
    let client = new_client(ptr::null());
    let key = unique_cstring("key");
    let mut err = new_err();
    let counter =
        unsafe { qortoo_counter_create(client.0, key.as_ptr(), null_options(), &mut err) };
    assert_ok(&mut err, "create");
    let counter = CounterGuard(counter);

    let got_key = read_and_free_string(unsafe { qortoo_counter_get_key(counter.0) });
    assert_eq!(got_key.as_deref(), key.to_str().ok());
    assert_eq!(
        unsafe { qortoo_counter_get_type(counter.0) },
        0 /* Counter */
    );
    assert_eq!(unsafe { qortoo_counter_get_client_version(counter.0) }, 0);
    assert_eq!(unsafe { qortoo_counter_get_server_version(counter.0) }, 0);
    assert_eq!(
        unsafe { qortoo_counter_get_synced_client_version(counter.0) },
        0
    );

    unsafe { qortoo_counter_increase(counter.0, &mut err) };
    assert_ok(&mut err, "increase");
    assert_eq!(unsafe { qortoo_counter_get_client_version(counter.0) }, 1);
}

// Two-client push/pull value transfer over `LocalConnectivity` is core CRDT/sync
// behavior, already proven directly in qortoo-rs (e.g.
// `datatypes::datatype::tests::can_unsubscribe_with_pending_transactions`). Re-running
// that scenario through the FFI wouldn't test anything specific to the C ABI beyond
// what the single-client tests in this file already cover (pointer marshalling,
// `qortoo_counter_sync`'s error-free path), so it isn't repeated here.

#[test]
fn can_transition_state_through_both_unsubscribe_paths() {
    let conn = manual_conn();
    let client = new_client(conn.0);
    let mut err = new_err();

    let counter_side = unsafe {
        qortoo_counter_create(
            client.0,
            unique_cstring("counter-side").as_ptr(),
            null_options(),
            &mut err,
        )
    };
    assert_ok(&mut err, "create (counter-driven)");
    let counter_side = CounterGuard(counter_side);
    unsafe { qortoo_counter_sync(counter_side.0, &mut err) };
    assert_ok(&mut err, "sync before counter.unsubscribe");
    unsafe { qortoo_counter_unsubscribe(counter_side.0, &mut err) };
    assert_ok(&mut err, "counter.unsubscribe");
    assert_eq!(
        unsafe { qortoo_counter_get_state(counter_side.0) },
        4 /* Unsubscribing */
    );

    let key_client_side = unique_cstring("client-side");
    let counter_client_side = unsafe {
        qortoo_counter_create(client.0, key_client_side.as_ptr(), null_options(), &mut err)
    };
    assert_ok(&mut err, "create (client-driven)");
    let counter_client_side = CounterGuard(counter_client_side);
    unsafe { qortoo_counter_sync(counter_client_side.0, &mut err) };
    assert_ok(&mut err, "sync before client.unsubscribe_datatype");
    unsafe { qortoo_client_unsubscribe_datatype(client.0, key_client_side.as_ptr(), &mut err) };
    assert_ok(&mut err, "client.unsubscribe_datatype");
    assert_eq!(
        unsafe { qortoo_counter_get_state(counter_client_side.0) },
        4 /* Unsubscribing */
    );
}

#[test]
fn can_reject_a_write_on_a_subscribing_counter_with_code_206() {
    // A `Subscribing` counter is not yet writable: `NotWritable`, not `ReadonlyViolation`
    // (see `can_reject_a_write_on_a_readonly_counter_with_code_207` for that split).
    let client = new_client(ptr::null());
    let mut err = new_err();
    let subscribing = unsafe {
        qortoo_counter_subscribe(
            client.0,
            unique_cstring("subscribing").as_ptr(),
            null_options(),
            &mut err,
        )
    };
    assert_ok(&mut err, "subscribe");
    let subscribing = CounterGuard(subscribing);
    unsafe { qortoo_counter_increase(subscribing.0, &mut err) };
    assert_err(
        &mut err,
        206, /* NotWritable */
        "write to subscribing counter",
    );
}

/// Every `qortoo_counter_*` call with no `err_out` parameter must return its
/// documented default on a null counter rather than crash — the getters below, plus
/// `unset_handler`'s `false` (a null counter never had anything registered to remove).
#[test]
fn can_return_documented_defaults_for_null_counter_calls() {
    assert_eq!(unsafe { qortoo_counter_get_value(ptr::null()) }, 0);
    assert_eq!(unsafe { qortoo_counter_get_state(ptr::null()) }, -1);
    assert_eq!(unsafe { qortoo_counter_get_type(ptr::null()) }, -1);
    assert!(unsafe { qortoo_counter_get_key(ptr::null()) }.is_null());
    assert_eq!(unsafe { qortoo_counter_get_server_version(ptr::null()) }, 0);
    assert_eq!(unsafe { qortoo_counter_get_client_version(ptr::null()) }, 0);
    assert_eq!(
        unsafe { qortoo_counter_get_synced_client_version(ptr::null()) },
        0
    );
    assert!(!unsafe { qortoo_counter_unset_handler(ptr::null(), 0) });
}

#[test]
fn can_reject_null_counter_fallible_operations_with_invalid_argument() {
    let mut err = new_err();

    let value = unsafe { qortoo_counter_increase(ptr::null(), &mut err) };
    assert_eq!(value, 0);
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "increase on null counter",
    );

    let value = unsafe { qortoo_counter_increase_by(ptr::null(), 3, &mut err) };
    assert_eq!(value, 0);
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "increase_by on null counter",
    );

    unsafe { qortoo_counter_sync(ptr::null(), &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "sync on null counter",
    );

    unsafe { qortoo_counter_sync_with_context(ptr::null(), ptr::null(), ptr::null(), &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "sync_with_context on null counter",
    );

    unsafe { qortoo_counter_unsubscribe(ptr::null(), &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "unsubscribe on null counter",
    );
}

#[test]
fn can_free_a_null_counter_safely() {
    unsafe { qortoo_counter_free(ptr::null_mut()) };
}

#[test]
fn can_accept_a_null_err_out_on_counter_operations() {
    let client = new_client(ptr::null());
    let counter = unsafe {
        qortoo_counter_create(
            client.0,
            unique_cstring("key").as_ptr(),
            null_options(),
            ptr::null_mut(),
        )
    };
    assert!(!counter.is_null());
    let counter = CounterGuard(counter);

    // Success path with a null `err_out`.
    let value = unsafe { qortoo_counter_increase(counter.0, ptr::null_mut()) };
    assert_eq!(value, 1);

    // Failure path with a null `err_out` (null counter): must not crash.
    let value = unsafe { qortoo_counter_increase(ptr::null(), ptr::null_mut()) };
    assert_eq!(value, 0);
}

#[test]
fn can_report_the_exact_error_from_both_unsubscribe_paths_once_disabled() {
    let conn = manual_conn();
    let client = new_client(conn.0);
    let key = unique_cstring("key");
    let mut err = new_err();
    let counter =
        unsafe { qortoo_counter_create(client.0, key.as_ptr(), null_options(), &mut err) };
    assert_ok(&mut err, "create");
    let counter = CounterGuard(counter);
    unsafe { qortoo_counter_sync(counter.0, &mut err) };
    assert_ok(&mut err, "sync (Creating -> Subscribed)");
    unsafe { qortoo_counter_unsubscribe(counter.0, &mut err) };
    assert_ok(&mut err, "unsubscribe (Subscribed -> Unsubscribing)");
    unsafe { qortoo_counter_sync(counter.0, &mut err) };
    assert_ok(&mut err, "sync (Unsubscribing -> Disabled)");

    // The datatype is now Disabled and detached from the client's manager: the
    // counter-driven path still sees the state directly (`NotWritable`), while the
    // client-driven path can no longer find the key at all (`Disallowed`) — both must
    // report their exact code rather than silently no-op.
    unsafe { qortoo_counter_unsubscribe(counter.0, &mut err) };
    assert_err(
        &mut err,
        206, /* NotWritable */
        "counter.unsubscribe once Disabled",
    );

    unsafe { qortoo_client_unsubscribe_datatype(client.0, key.as_ptr(), &mut err) };
    assert_err(
        &mut err,
        205, /* Disallowed: no longer managed by this client */
        "client.unsubscribe_datatype once Disabled",
    );
}
