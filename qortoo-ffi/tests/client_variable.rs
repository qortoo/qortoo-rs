//! `Variable` handle exercised through the C ABI: that every new `qortoo_variable_*`
//! symbol is callable, reports the Variable datatype, is null-safe, and — for the raw JSON
//! entry points — preserves the caller's bytes and owns the returned buffers correctly.
//!
//! Construction gates (readonly, state, duplicate keys), the transaction callback
//! bridge, handler userdata ownership, and LWW/convergence semantics are
//! datatype-agnostic or core concerns already proven by `client_counter.rs`,
//! `transaction.rs`, `handler.rs`, and the qortoo-rs suite — so they are not re-tested
//! per datatype here.

mod support;

use std::ptr;

use qortoo_ffi::{
    QORTOO_ERR_INVALID_ARGUMENT, QortooOwnedBytes, QortooVariable, qortoo_client_new,
    qortoo_datatype_get_client_version, qortoo_datatype_get_key,
    qortoo_datatype_get_server_version, qortoo_datatype_get_state,
    qortoo_datatype_get_synced_client_version, qortoo_datatype_get_type,
    qortoo_datatype_set_handler, qortoo_datatype_sync, qortoo_datatype_sync_with_context,
    qortoo_datatype_unset_handler, qortoo_datatype_unsubscribe, qortoo_owned_bytes_free,
    qortoo_variable_as_datatype, qortoo_variable_create, qortoo_variable_free,
    qortoo_variable_get_raw, qortoo_variable_set_raw, qortoo_variable_subscribe,
    qortoo_variable_subscribe_or_create, qortoo_variable_transaction,
    qortoo_variable_transaction_with_context,
};
use rstest::rstest;
use support::*;

/// A `{NULL, 0}` owned-bytes out-parameter.
fn empty_owned_bytes() -> QortooOwnedBytes {
    QortooOwnedBytes {
        data: ptr::null_mut(),
        len: 0,
    }
}

/// Copies an owned-bytes buffer to a `String` and frees it exactly once.
fn take_owned_bytes(bytes: QortooOwnedBytes) -> String {
    let copied = if bytes.data.is_null() {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(bytes.data, bytes.len) }.to_vec()
    };
    unsafe { qortoo_owned_bytes_free(bytes) };
    String::from_utf8(copied).expect("owned JSON bytes are UTF-8")
}

/// `qortoo_variable_set_raw` on the success path, returning the previous value's JSON.
fn set_raw(variable: *const QortooVariable, json: &str) -> String {
    let mut previous = empty_owned_bytes();
    let mut err = new_err();
    unsafe {
        qortoo_variable_set_raw(variable, json.as_ptr(), json.len(), &mut previous, &mut err)
    };
    assert_ok(&mut err, "set_raw");
    take_owned_bytes(previous)
}

/// `qortoo_variable_get_raw` on the success path, returning the current value's JSON.
fn get_raw(variable: *const QortooVariable) -> String {
    let mut value = empty_owned_bytes();
    let mut err = new_err();
    unsafe { qortoo_variable_get_raw(variable, &mut value, &mut err) };
    assert_ok(&mut err, "get_raw");
    take_owned_bytes(value)
}

fn new_client() -> ClientGuard {
    let collection = unique_cstring("collection");
    let alias = unique_cstring("alias");
    let mut err = new_err();
    let client =
        unsafe { qortoo_client_new(collection.as_ptr(), alias.as_ptr(), ptr::null(), &mut err) };
    assert_ok(&mut err, "client creation");
    ClientGuard(client)
}

fn create_variable(client: &ClientGuard, key_prefix: &str) -> VariableGuard {
    let mut err = new_err();
    let variable = unsafe {
        qortoo_variable_create(
            client.0,
            unique_cstring(key_prefix).as_ptr(),
            ptr::null(),
            &mut err,
        )
    };
    assert_ok(&mut err, "variable creation");
    assert!(!variable.is_null());
    VariableGuard(variable)
}

#[test]
fn can_construct_variables_with_the_expected_initial_state_per_mode() {
    let client = new_client();
    let mut err = new_err();

    let created = unsafe {
        qortoo_variable_create(
            client.0,
            unique_cstring("create").as_ptr(),
            ptr::null(),
            &mut err,
        )
    };
    assert_ok(&mut err, "create");
    let created = VariableGuard(created);
    assert_eq!(
        unsafe { qortoo_datatype_get_state(qortoo_variable_as_datatype(created.0)) },
        0 /* Creating */
    );

    let subscribed = unsafe {
        qortoo_variable_subscribe(
            client.0,
            unique_cstring("subscribe").as_ptr(),
            ptr::null(),
            &mut err,
        )
    };
    assert_ok(&mut err, "subscribe");
    let subscribed = VariableGuard(subscribed);
    assert_eq!(
        unsafe { qortoo_datatype_get_state(qortoo_variable_as_datatype(subscribed.0)) },
        1 /* Subscribing */
    );

    let sub_or_create = unsafe {
        qortoo_variable_subscribe_or_create(
            client.0,
            unique_cstring("subscribe-or-create").as_ptr(),
            ptr::null(),
            &mut err,
        )
    };
    assert_ok(&mut err, "subscribe_or_create");
    let sub_or_create = VariableGuard(sub_or_create);
    assert_eq!(
        unsafe { qortoo_datatype_get_state(qortoo_variable_as_datatype(sub_or_create.0)) },
        2 /* SubscribingOrCreating */
    );
}

#[test]
fn can_report_the_key_and_variable_type_through_the_getters() {
    let client = new_client();
    let key = unique_cstring("key");
    let mut err = new_err();
    let variable = unsafe { qortoo_variable_create(client.0, key.as_ptr(), ptr::null(), &mut err) };
    assert_ok(&mut err, "create");
    let variable = VariableGuard(variable);

    let got_key = read_and_free_string(unsafe {
        qortoo_datatype_get_key(qortoo_variable_as_datatype(variable.0))
    });
    assert_eq!(got_key.as_deref(), key.to_str().ok());
    assert_eq!(
        unsafe { qortoo_datatype_get_type(qortoo_variable_as_datatype(variable.0)) },
        1, /* Variable */
    );
    assert_eq!(
        unsafe { qortoo_datatype_get_client_version(qortoo_variable_as_datatype(variable.0)) },
        0
    );
    assert_eq!(
        unsafe { qortoo_datatype_get_server_version(qortoo_variable_as_datatype(variable.0)) },
        0
    );
    assert_eq!(
        unsafe {
            qortoo_datatype_get_synced_client_version(qortoo_variable_as_datatype(variable.0))
        },
        0
    );
}

extern "C" fn commit_callback(_tx: *mut QortooVariable, _userdata: usize) -> i32 {
    0
}

extern "C" fn abort_callback(_tx: *mut QortooVariable, _userdata: usize) -> i32 {
    1
}

#[test]
fn can_run_both_transaction_entry_points_through_the_callback_bridge() {
    let client = new_client();
    let variable = create_variable(&client, "key");
    let tag = unique_cstring("tag");
    let mut err = new_err();

    unsafe { qortoo_variable_transaction(variable.0, tag.as_ptr(), commit_callback, 0, &mut err) };
    assert_ok(&mut err, "committed transaction");

    unsafe { qortoo_variable_transaction(variable.0, tag.as_ptr(), abort_callback, 0, &mut err) };
    assert_err(
        &mut err,
        201, /* TransactionFailed */
        "aborted transaction",
    );

    unsafe {
        qortoo_variable_transaction_with_context(
            variable.0,
            tag.as_ptr(),
            ptr::null(),
            ptr::null(),
            commit_callback,
            0,
            &mut err,
        )
    };
    assert_ok(&mut err, "committed transaction with context");
}

extern "C" fn noop_on_state_change(_userdata: usize, _old: i32, _new: i32) {}

#[test]
fn can_register_and_unregister_a_variable_handler() {
    let client = new_client();
    let variable = create_variable(&client, "key");

    unsafe {
        qortoo_datatype_set_handler(
            qortoo_variable_as_datatype(variable.0),
            3,
            Some(noop_on_state_change),
            None,
            0,
            None,
        )
    };
    assert!(
        unsafe { qortoo_datatype_unset_handler(qortoo_variable_as_datatype(variable.0), 3) },
        "the handler registered at priority 3 must be removable"
    );
    assert!(
        !unsafe { qortoo_datatype_unset_handler(qortoo_variable_as_datatype(variable.0), 3) },
        "a second unset at the same priority removes nothing"
    );
}

#[test]
fn can_return_documented_defaults_for_null_variable_calls() {
    assert_eq!(
        unsafe { qortoo_datatype_get_state(qortoo_variable_as_datatype(ptr::null_mut())) },
        -1
    );
    assert_eq!(
        unsafe { qortoo_datatype_get_type(qortoo_variable_as_datatype(ptr::null_mut())) },
        -1
    );
    assert!(
        unsafe { qortoo_datatype_get_key(qortoo_variable_as_datatype(ptr::null_mut())) }.is_null()
    );
    assert_eq!(
        unsafe { qortoo_datatype_get_server_version(qortoo_variable_as_datatype(ptr::null_mut())) },
        0
    );
    assert_eq!(
        unsafe { qortoo_datatype_get_client_version(qortoo_variable_as_datatype(ptr::null_mut())) },
        0
    );
    assert_eq!(
        unsafe {
            qortoo_datatype_get_synced_client_version(qortoo_variable_as_datatype(ptr::null_mut()))
        },
        0
    );
    assert!(!unsafe {
        qortoo_datatype_unset_handler(qortoo_variable_as_datatype(ptr::null_mut()), 0)
    });
}

#[test]
fn can_reject_null_variable_fallible_operations_with_invalid_argument() {
    let mut err = new_err();

    let created =
        unsafe { qortoo_variable_create(ptr::null(), ptr::null(), ptr::null(), &mut err) };
    assert!(created.is_null());
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "create on null client",
    );

    unsafe { qortoo_datatype_sync(qortoo_variable_as_datatype(ptr::null_mut()), &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "sync on null variable",
    );

    unsafe {
        qortoo_datatype_sync_with_context(
            qortoo_variable_as_datatype(ptr::null_mut()),
            ptr::null(),
            ptr::null(),
            &mut err,
        )
    };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "sync_with_context on null variable",
    );

    unsafe { qortoo_datatype_unsubscribe(qortoo_variable_as_datatype(ptr::null_mut()), &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "unsubscribe on null variable",
    );

    unsafe { qortoo_variable_transaction(ptr::null(), ptr::null(), commit_callback, 0, &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "transaction on null variable",
    );

    unsafe {
        qortoo_variable_transaction_with_context(
            ptr::null(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            commit_callback,
            0,
            &mut err,
        )
    };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "transaction_with_context on null variable",
    );
}

#[test]
fn can_free_a_null_variable_safely() {
    unsafe { qortoo_variable_free(ptr::null_mut()) };
}

// ---------------------------------------------------------------------------
// JSON value
// ---------------------------------------------------------------------------

#[test]
fn can_read_the_initial_value_as_json_null() {
    let client = new_client();
    let variable = create_variable(&client, "key");
    assert_eq!(get_raw(variable.0), "null");
}

#[test]
fn can_round_trip_json_and_return_the_previous_value() {
    let client = new_client();
    let variable = create_variable(&client, "key");

    // The first set reports the initial JSON null; the stored value keeps the
    // caller's bytes — object key order included — rather than canonicalizing them.
    assert_eq!(set_raw(variable.0, r#"{"b":2,"a":1}"#), "null");
    assert_eq!(get_raw(variable.0), r#"{"b":2,"a":1}"#);

    // A later set of a different JSON type returns the previous value verbatim.
    assert_eq!(set_raw(variable.0, "[1,2,3]"), r#"{"b":2,"a":1}"#);
    assert_eq!(get_raw(variable.0), "[1,2,3]");
}

#[rstest]
#[case::empty(b"")]
#[case::truncated(b"{\"a\":")]
#[case::two_values(b"1 2")]
#[case::invalid_utf8(b"\"\xff\"")]
fn can_reject_invalid_json_with_code_214(#[case] bad_json: &[u8]) {
    let client = new_client();
    let variable = create_variable(&client, "key");

    let mut previous = empty_owned_bytes();
    let mut err = new_err();
    unsafe {
        qortoo_variable_set_raw(
            variable.0,
            bad_json.as_ptr(),
            bad_json.len(),
            &mut previous,
            &mut err,
        )
    };
    assert_err(&mut err, 214, "invalid JSON set");
    assert!(
        previous.data.is_null(),
        "previous_out must stay the sentinel"
    );
    assert_eq!(
        get_raw(variable.0),
        "null",
        "the variable must be unchanged"
    );
}

#[test]
fn can_reject_null_arguments_on_the_raw_entry_points() {
    let client = new_client();
    let variable = create_variable(&client, "key");
    let json = "1";
    let mut out = empty_owned_bytes();
    let mut err = new_err();

    unsafe { qortoo_variable_set_raw(ptr::null(), json.as_ptr(), json.len(), &mut out, &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "set_raw on null variable",
    );

    unsafe { qortoo_variable_set_raw(variable.0, ptr::null(), 0, &mut out, &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "set_raw with null data",
    );

    unsafe {
        qortoo_variable_set_raw(
            variable.0,
            json.as_ptr(),
            json.len(),
            ptr::null_mut(),
            &mut err,
        )
    };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "set_raw with null previous_out",
    );

    unsafe { qortoo_variable_get_raw(ptr::null(), &mut out, &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "get_raw on null variable",
    );

    unsafe { qortoo_variable_get_raw(variable.0, ptr::null_mut(), &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "get_raw with null value_out",
    );
}

#[test]
fn can_leave_previous_out_untouched_when_a_set_is_not_writable() {
    let client = new_client();
    let mut err = new_err();
    let subscribing = unsafe {
        qortoo_variable_subscribe(
            client.0,
            unique_cstring("key").as_ptr(),
            ptr::null(),
            &mut err,
        )
    };
    assert_ok(&mut err, "subscribe");
    let subscribing = VariableGuard(subscribing);

    let json = "42";
    let mut previous = empty_owned_bytes();
    unsafe {
        qortoo_variable_set_raw(
            subscribing.0,
            json.as_ptr(),
            json.len(),
            &mut previous,
            &mut err,
        )
    };
    assert_err(
        &mut err,
        206, /* NotWritable */
        "set_raw while subscribing",
    );
    assert!(
        previous.data.is_null(),
        "previous_out must stay the sentinel"
    );
}

#[test]
fn can_read_an_explicit_null_and_a_non_ascii_value_verbatim() {
    let client = new_client();
    let variable = create_variable(&client, "key");

    // An explicit `set` of JSON null reads back exactly like the initial value.
    assert_eq!(set_raw(variable.0, "null"), "null");
    assert_eq!(get_raw(variable.0), "null");

    // Non-ASCII UTF-8 inside a JSON string survives the round trip byte for byte.
    let value = r#"{"greeting":"안녕하세요"}"#;
    set_raw(variable.0, value);
    assert_eq!(get_raw(variable.0), value);
}

static TX_SET_ERR_CODE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

extern "C" fn set_inside_transaction(tx: *mut QortooVariable, _userdata: usize) -> i32 {
    let json = "[1,2,3]";
    let mut previous = empty_owned_bytes();
    let mut err = new_err();
    unsafe { qortoo_variable_set_raw(tx, json.as_ptr(), json.len(), &mut previous, &mut err) };
    TX_SET_ERR_CODE.store(err.code, std::sync::atomic::Ordering::SeqCst);
    unsafe { qortoo_owned_bytes_free(previous) };
    if err.msg.is_null() { 0 } else { 1 }
}

#[test]
fn can_set_a_value_through_the_borrowed_transaction_handle() {
    let client = new_client();
    let variable = create_variable(&client, "key");
    let tag = unique_cstring("tag");
    let mut err = new_err();

    unsafe {
        qortoo_variable_transaction(
            variable.0,
            tag.as_ptr(),
            set_inside_transaction,
            0,
            &mut err,
        )
    };
    assert_ok(&mut err, "committed transaction with a set");
    assert_eq!(
        TX_SET_ERR_CODE.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "set_raw on the borrowed transaction handle must succeed"
    );
    assert_eq!(get_raw(variable.0), "[1,2,3]");
}
