//! Transaction commit/rollback and trace-context propagation, exercised through the C
//! ABI callback bridge (`qortoo_counter_transaction[_with_context]`).

mod support;

use std::ptr;

use qortoo_ffi::{
    QortooCounter, qortoo_client_new, qortoo_counter_create, qortoo_counter_get_value,
    qortoo_counter_increase_by, qortoo_counter_transaction,
    qortoo_counter_transaction_with_context,
};
use rstest::rstest;
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

fn new_counter(client: &ClientGuard) -> CounterGuard {
    let mut err = new_err();
    let counter = unsafe {
        qortoo_counter_create(
            client.0,
            unique_cstring("key").as_ptr(),
            ptr::null(),
            &mut err,
        )
    };
    assert_ok(&mut err, "counter creation");
    CounterGuard(counter)
}

// ---------------------------------------------------------------------------
// Commit
// ---------------------------------------------------------------------------

extern "C" fn commit_two_increases(tx_counter: *mut QortooCounter, _userdata: usize) -> i32 {
    let mut err = new_err();
    unsafe { qortoo_counter_increase_by(tx_counter, 4, &mut err) };
    assert_ok(&mut err, "increase_by inside transaction (1)");
    unsafe { qortoo_counter_increase_by(tx_counter, 6, &mut err) };
    assert_ok(&mut err, "increase_by inside transaction (2)");
    0
}

#[test]
fn can_commit_multiple_operations_in_a_transaction() {
    let client = new_client();
    let counter = new_counter(&client);
    let tag = unique_cstring("tag");
    let mut err = new_err();

    unsafe {
        qortoo_counter_transaction(counter.0, tag.as_ptr(), commit_two_increases, 0, &mut err)
    };
    assert_ok(&mut err, "transaction commit");
    assert_eq!(unsafe { qortoo_counter_get_value(counter.0) }, 10);
}

// ---------------------------------------------------------------------------
// Rollback
// ---------------------------------------------------------------------------

extern "C" fn abort_after_write(tx_counter: *mut QortooCounter, _userdata: usize) -> i32 {
    let mut err = new_err();
    unsafe { qortoo_counter_increase_by(tx_counter, 100, &mut err) };
    assert_ok(&mut err, "increase_by before abort");
    7 // any non-zero code rolls back
}

#[test]
fn can_roll_back_on_a_non_zero_callback_result() {
    let client = new_client();
    let counter = new_counter(&client);
    let tag = unique_cstring("tag");
    let mut err = new_err();

    unsafe { qortoo_counter_increase_by(counter.0, 1, &mut err) };
    assert_ok(&mut err, "seed value");

    unsafe { qortoo_counter_transaction(counter.0, tag.as_ptr(), abort_after_write, 0, &mut err) };
    assert_err(
        &mut err,
        201, /* TransactionFailed */
        "aborted transaction",
    );
    assert_eq!(
        unsafe { qortoo_counter_get_value(counter.0) },
        1,
        "the write inside the aborted transaction must be rolled back"
    );
}

// ---------------------------------------------------------------------------
// Userdata
// ---------------------------------------------------------------------------

extern "C" fn record_userdata(_tx_counter: *mut QortooCounter, userdata: usize) -> i32 {
    let slot = userdata as *const std::sync::atomic::AtomicUsize;
    unsafe { &*slot }.store(userdata, std::sync::atomic::Ordering::SeqCst);
    0
}

#[test]
fn can_forward_userdata_unchanged_to_the_transaction_callback() {
    let client = new_client();
    let counter = new_counter(&client);
    let tag = unique_cstring("tag");
    let mut err = new_err();

    let slot: &'static std::sync::atomic::AtomicUsize =
        Box::leak(Box::new(std::sync::atomic::AtomicUsize::new(0)));
    let userdata = slot as *const _ as usize;

    unsafe {
        qortoo_counter_transaction(counter.0, tag.as_ptr(), record_userdata, userdata, &mut err)
    };
    assert_ok(&mut err, "transaction with userdata");
    assert_eq!(slot.load(std::sync::atomic::Ordering::SeqCst), userdata);
}

// ---------------------------------------------------------------------------
// Argument validation
// ---------------------------------------------------------------------------

extern "C" fn unreachable_callback(_tx_counter: *mut QortooCounter, _userdata: usize) -> i32 {
    panic!("callback must not run when argument validation rejects the call first");
}

/// Which transaction entry point a `can_reject_a_bad_transaction_argument` case calls.
#[derive(Clone, Copy)]
enum Entry {
    Plain,
    WithContext,
}

#[derive(Clone, Copy)]
enum BadTxArg {
    NullCounter,
    NullTag,
    InvalidUtf8Tag,
}

#[rstest]
#[case::plain_null_counter(Entry::Plain, BadTxArg::NullCounter)]
#[case::plain_null_tag(Entry::Plain, BadTxArg::NullTag)]
#[case::plain_invalid_utf8_tag(Entry::Plain, BadTxArg::InvalidUtf8Tag)]
#[case::with_context_null_counter(Entry::WithContext, BadTxArg::NullCounter)]
#[case::with_context_null_tag(Entry::WithContext, BadTxArg::NullTag)]
#[case::with_context_invalid_utf8_tag(Entry::WithContext, BadTxArg::InvalidUtf8Tag)]
fn can_reject_a_bad_transaction_argument(#[case] entry: Entry, #[case] bad: BadTxArg) {
    let client = new_client();
    let counter = new_counter(&client);
    let good_tag = unique_cstring("tag");
    let invalid = InvalidUtf8CString::new();
    let (counter_ptr, tag_ptr) = match bad {
        BadTxArg::NullCounter => (ptr::null(), good_tag.as_ptr()),
        BadTxArg::NullTag => (counter.0 as *const _, ptr::null()),
        BadTxArg::InvalidUtf8Tag => (counter.0 as *const _, invalid.as_ptr()),
    };

    let mut err = new_err();
    match entry {
        Entry::Plain => unsafe {
            qortoo_counter_transaction(counter_ptr, tag_ptr, unreachable_callback, 0, &mut err)
        },
        Entry::WithContext => unsafe {
            qortoo_counter_transaction_with_context(
                counter_ptr,
                tag_ptr,
                ptr::null(),
                ptr::null(),
                unreachable_callback,
                0,
                &mut err,
            )
        },
    };
    assert_err(
        &mut err,
        qortoo_ffi::QORTOO_ERR_INVALID_ARGUMENT,
        "bad transaction argument",
    );
}

// ---------------------------------------------------------------------------
// Trace-context variant
// ---------------------------------------------------------------------------

const TRACEPARENT: &str = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";

#[rstest]
#[case::absent_headers(None)]
#[case::malformed_headers(Some("garbage"))]
#[case::valid_headers(Some(TRACEPARENT))]
fn can_accept_trace_headers_in_a_transaction(#[case] traceparent: Option<&str>) {
    let client = new_client();
    let counter = new_counter(&client);
    let tag = unique_cstring("tag");
    let traceparent_c = traceparent.map(|s| std::ffi::CString::new(s).unwrap());
    let traceparent_ptr = traceparent_c.as_ref().map_or(ptr::null(), |c| c.as_ptr());
    let mut err = new_err();

    unsafe {
        qortoo_counter_transaction_with_context(
            counter.0,
            tag.as_ptr(),
            traceparent_ptr,
            ptr::null(),
            commit_two_increases,
            0,
            &mut err,
        )
    };
    assert_ok(&mut err, "transaction with trace headers");
    assert_eq!(unsafe { qortoo_counter_get_value(counter.0) }, 10);
}

// A dedicated "the tx-scoped handle is usable during the callback" test was removed:
// every commit/rollback test above already writes through that same handle, which is a
// strictly stronger proof of liveness than a read-only variant would add. Exercising
// "only during" (i.e. retaining or using it after the callback returns) would be a
// use-after-free, which is deliberately out of scope for this crate's tests.
