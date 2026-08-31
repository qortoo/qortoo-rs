//! `Variable` handle lifecycle exercised through the C ABI: that every new
//! `qortoo_variable_*` symbol is callable, reports the Variable datatype, and is
//! null-safe.
//!
//! Construction gates (readonly, state, duplicate keys), the transaction callback
//! bridge, and handler userdata ownership are datatype-agnostic — they run through the
//! same shared plumbing `client_counter.rs`, `transaction.rs`, and `handler.rs` already
//! prove — so they are not re-tested per datatype here. The value `set`/`get` entry
//! points are added separately.

mod support;

use std::ptr;

use qortoo_ffi::{
    QORTOO_ERR_INVALID_ARGUMENT, QortooVariable, qortoo_client_new, qortoo_variable_create,
    qortoo_variable_free, qortoo_variable_get_client_version, qortoo_variable_get_key,
    qortoo_variable_get_server_version, qortoo_variable_get_state,
    qortoo_variable_get_synced_client_version, qortoo_variable_get_type,
    qortoo_variable_set_handler, qortoo_variable_subscribe, qortoo_variable_subscribe_or_create,
    qortoo_variable_sync, qortoo_variable_sync_with_context, qortoo_variable_transaction,
    qortoo_variable_transaction_with_context, qortoo_variable_unset_handler,
    qortoo_variable_unsubscribe,
};
use support::*;

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
        unsafe { qortoo_variable_get_state(created.0) },
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
        unsafe { qortoo_variable_get_state(subscribed.0) },
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
        unsafe { qortoo_variable_get_state(sub_or_create.0) },
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

    let got_key = read_and_free_string(unsafe { qortoo_variable_get_key(variable.0) });
    assert_eq!(got_key.as_deref(), key.to_str().ok());
    assert_eq!(
        unsafe { qortoo_variable_get_type(variable.0) },
        1, /* Variable */
    );
    assert_eq!(unsafe { qortoo_variable_get_client_version(variable.0) }, 0);
    assert_eq!(unsafe { qortoo_variable_get_server_version(variable.0) }, 0);
    assert_eq!(
        unsafe { qortoo_variable_get_synced_client_version(variable.0) },
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
        qortoo_variable_set_handler(variable.0, 3, Some(noop_on_state_change), None, 0, None)
    };
    assert!(
        unsafe { qortoo_variable_unset_handler(variable.0, 3) },
        "the handler registered at priority 3 must be removable"
    );
    assert!(
        !unsafe { qortoo_variable_unset_handler(variable.0, 3) },
        "a second unset at the same priority removes nothing"
    );
}

#[test]
fn can_return_documented_defaults_for_null_variable_calls() {
    assert_eq!(unsafe { qortoo_variable_get_state(ptr::null()) }, -1);
    assert_eq!(unsafe { qortoo_variable_get_type(ptr::null()) }, -1);
    assert!(unsafe { qortoo_variable_get_key(ptr::null()) }.is_null());
    assert_eq!(
        unsafe { qortoo_variable_get_server_version(ptr::null()) },
        0
    );
    assert_eq!(
        unsafe { qortoo_variable_get_client_version(ptr::null()) },
        0
    );
    assert_eq!(
        unsafe { qortoo_variable_get_synced_client_version(ptr::null()) },
        0
    );
    assert!(!unsafe { qortoo_variable_unset_handler(ptr::null(), 0) });
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

    unsafe { qortoo_variable_sync(ptr::null(), &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "sync on null variable",
    );

    unsafe { qortoo_variable_sync_with_context(ptr::null(), ptr::null(), ptr::null(), &mut err) };
    assert_err(
        &mut err,
        QORTOO_ERR_INVALID_ARGUMENT,
        "sync_with_context on null variable",
    );

    unsafe { qortoo_variable_unsubscribe(ptr::null(), &mut err) };
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
