//! Product-authored return-error wording shared by the Rust SDK and C ABI.
//!
//! Keep external causes, logs, raw panics, and test expectations at their call sites.
//! Follow the catalog ownership and naming rules in `docs/error-handling.md`.

// Each crate and feature set uses a different subset of this shared catalog.
#![allow(dead_code)]

pub(crate) mod client {
    use std::fmt::{Debug, Display};

    pub(crate) const INVALID_COLLECTION_NAME_PREFIX: &str = "invalid collection name: ";

    pub(crate) const SUBSCRIBE_OR_CREATE_FAILED_PREFIX: &str =
        "cannot subscribe or create datatype: ";

    pub(crate) const INVALID_COLLECTION_NAME_TEMPLATE: &str = "'{}': collection name must be 1-47 bytes, start with an ASCII letter or underscore, contain only ASCII letters, digits, '.', '_', '~', or '-', and must not start with 'system.' or contain '.system.'";

    pub(crate) const INVALID_DATATYPE_KEY_TEMPLATE: &str = "invalid datatype key '{}': must be 1-255 bytes, must not contain null characters (\\0), and must not start with '$'";

    pub(crate) fn datatype_not_managed(key: impl Display) -> String {
        format!("datatype '{key}' is not managed by this client")
    }

    pub(crate) fn datatype_already_managed(
        datatype_type: impl Debug,
        key: impl Display,
        state: impl Debug,
        existing_type: impl Debug,
    ) -> String {
        format!(
            "cannot register {datatype_type:?} '{key}' in {state:?} state: this client already manages the key as {existing_type:?}"
        )
    }
}

pub(crate) mod datatype {
    use std::fmt::{Debug, Display};

    pub(crate) const EXECUTE_OPERATION_PREFIX: &str = "execute operation: ";

    pub(crate) const TRANSACTION_FAILED_PREFIX: &str = "transaction failed: ";

    pub(crate) const INTERNAL_PREFIX: &str = "internal error: ";

    pub(crate) const DISALLOWED_PREFIX: &str = "operation not allowed: ";

    pub(crate) const NOT_WRITABLE_PREFIX: &str = "not writable: ";

    pub(crate) const READONLY_VIOLATION: &str = "readonly violation";

    pub(crate) const PUSH_BUFFER_EXCEEDED_MAX_MEM_SIZE: &str = "push buffer memory limit exceeded";

    pub(crate) const SYNC_DISABLED: &str = "sync is not allowed in Disabled state";

    pub(crate) const SYNC_ON_TRANSACTION_OWNER: &str =
        "sync is not allowed on the thread that owns the active local transaction";

    pub(crate) const REENTRANT_TRANSACTION: &str = "cannot start another local transaction on the thread that owns the active transaction; use the callback handle for writes";

    pub(crate) const NESTED_TRANSACTION: &str =
        "nested transactions on the same datatype are not allowed";

    pub(crate) const COUNTER_LOCAL_OPERATION: &str = "counter cannot execute this local operation";

    pub(crate) const COUNTER_REMOTE_OPERATION: &str =
        "counter cannot execute this remote operation";

    pub(crate) const ROLLBACK_ACTION_MISMATCH: &str =
        "rollback action does not match the CRDT type";

    pub(crate) const VARIABLE_LOCAL_OPERATION: &str =
        "variable cannot execute this local operation";

    pub(crate) const VARIABLE_REMOTE_OPERATION: &str =
        "variable cannot execute this remote operation";

    pub(crate) const VARIABLE_TIMESTAMP_NOT_ADVANCED: &str =
        "local variable set did not advance the winning timestamp";

    pub(crate) const POSITIVE_LAMPORT_REQUIRED: &str =
        "modification operation must use a positive Lamport timestamp";

    pub(crate) const UNEXPECTED_RETURN_TYPE: &str = "unexpected return type";

    pub(crate) const UNSUBSCRIBE_OPERATION: &str = "unsubscribe";

    pub(crate) fn not_writable(key: impl Display, state: impl Debug) -> String {
        format!("cannot modify datatype '{key}' in {state:?} state")
    }

    pub(crate) fn operation_requires_subscription(
        operation: impl Display,
        state: impl Debug,
    ) -> String {
        format!("{operation} is only allowed in Subscribed state, current state is {state:?}")
    }

    pub(crate) fn conflicting_variable_timestamp(timestamp: impl Display) -> String {
        format!("variable received different values for timestamp {timestamp}")
    }
}

pub(crate) mod sync {
    use std::fmt::{Debug, Display};

    pub(crate) const EVENT_LOOP_PREFIX: &str = "event loop: ";

    pub(crate) const NON_SEQUENTIAL_CSEQ: &str = "non-sequential cseq in push buffer";

    pub(crate) const GET_PUSHING_TRANSACTIONS_FAILED: &str = "failed to get pushing transactions";

    pub(crate) const SYNC_FAILED_PREFIX: &str = "sync failed: ";

    pub(crate) const SERVER_REJECTED_PREFIX: &str = "server rejected: ";

    pub(crate) const TIMED_OUT_PREFIX: &str = "connection timed out: ";

    pub(crate) const PROTOCOL_VIOLATION_PREFIX: &str = "protocol violation: ";

    pub(crate) const READONLY_VIOLATION: &str = "readonly client attempted write operation";

    pub(crate) const CREATE_FAILED_PREFIX: &str = "creation failed: ";

    pub(crate) const RESOURCE_NOT_FOUND_PREFIX: &str = "resource not found: ";

    pub(crate) const MISSING_SUBSCRIPTION_PREFIX: &str = "missing subscription: ";

    pub(crate) const SERVER_INTERNAL_ERROR_PREFIX: &str = "server internal error: ";

    pub(crate) const INVALID_GC_ACK: &str = "GC acknowledgement exceeds applied checkpoint";

    pub(crate) const INVALID_TRANSACTION: &str = "invalid transaction identity or client sequence";

    pub(crate) const INVALID_IDENTITY: &str = "invalid datatype identity";

    pub(crate) const INVALID_CHECKPOINT: &str = "checkpoint exceeds the server head";

    pub(crate) const HISTORY_PRUNED: &str = "requested server history has been pruned";

    pub(crate) const ALREADY_EXISTS: &str = "datatype already exists";

    pub(crate) const INVALID_CREATION: &str = "invalid creation DUID or type";

    pub(crate) const DISABLED_PUSH: &str = "disabled client attempted push";

    pub(crate) const DELETING_UNSUPPORTED: &str = "datatype deletion is not supported";

    pub(crate) const SUBSCRIBING_PUSH: &str = "cannot push transactions when subscribing";

    pub(crate) const INVALID_SUBSCRIPTION_CHECKPOINT: &str = "invalid subscription checkpoint";

    pub(crate) const EVENT_LOOP_STOPPED: &str = "event loop stopped";

    pub(crate) const RESUBSCRIBE_REQUIRES_SNAPSHOT: &str = "resubscription requires a snapshot";

    pub(crate) const SUBSCRIBE_WITH_TRANSACTIONS: &str = "cannot subscribe with transactions";

    pub(crate) const CREATION_REQUIRES_SUBSCRIPTION: &str =
        "creation retry requires an active subscription generation";

    pub(crate) const RETIRED_SUBSCRIPTION: &str = "retired subscription generation";

    pub(crate) const SUBSCRIPTION_GENERATION_MISMATCH: &str = "subscription generation mismatch";

    pub(crate) const UNBOUNDED_CHANNEL: &str = "unbounded";

    pub(crate) const BOUNDED_CHANNEL: &str = "bounded";

    pub(crate) fn invalid_response_checkpoint(
        new: impl Display,
        old: impl Display,
        sent_cseq: impl Display,
    ) -> String {
        format!("invalid response checkpoint {new}; previous {old}, sent cseq {sent_cseq}")
    }

    pub(crate) fn illegal_response_state(new: impl Display, old: impl Display) -> String {
        format!("illegal state from push-pull: received {new} for {old}")
    }

    pub(crate) fn event_channel_stopped(channel: impl Display, error: impl Debug) -> String {
        format!("{channel} channel error {error:?}; stopping event loop")
    }

    pub(crate) fn response_failed(error: impl Display) -> String {
        format!("failed to receive sync() response: {error}")
    }

    pub(crate) fn missing_active_subscription(cuid: impl Display) -> String {
        format!("client '{cuid}' has no active datatype subscription on this server")
    }

    pub(crate) fn resource_not_found(
        datatype_type: impl Display,
        resource_id: impl Display,
    ) -> String {
        format!("{datatype_type} '{resource_id}' does not exist")
    }

    pub(crate) fn resource_type_mismatch(
        resource_id: impl Display,
        pushed_type: impl Display,
        existing_type: impl Display,
    ) -> String {
        format!(
            "datatype type mismatch for '{resource_id}': requested {pushed_type}, existing {existing_type}"
        )
    }
}

pub(crate) mod snapshot {
    use std::fmt::Display;

    pub(crate) const DESERIALIZE_PREFIX: &str = "deserialize: ";

    pub(crate) const COUNTER_CONTEXT: &str = "counter crdt";

    pub(crate) const VARIABLE_CONTEXT: &str = "variable crdt";

    pub(crate) const JSON_LENGTH_OVERFLOW: &str = "JSON length does not fit this platform";

    pub(crate) const INVALID_INITIAL_VARIABLE: &str =
        "Lamport zero is reserved for the initial null state";

    pub(crate) const UID_CONTEXT: &str = "uid";

    pub(crate) const TIMESTAMP_CONTEXT: &str = "timestamp";

    pub(crate) const INVALID_UTF8: &str = "not valid UTF-8";

    pub(crate) const INVALID_UID_DETAIL: &str = "invalid format";

    pub(crate) const INVALID_UID_FORMAT: &str =
        "invalid UID format: expected 16 ASCII letters, digits, '-' or '_'";

    pub(crate) const VERSION_FIELD: &str = "version";

    pub(crate) const TIMESTAMP_FIELD: &str = "winning timestamp";

    pub(crate) const JSON_LENGTH_FIELD: &str = "JSON length";

    pub(crate) const JSON_PAYLOAD_FIELD: &str = "JSON payload";

    pub(crate) const NO_SNAPSHOT_OPERATION: &str = "no snapshot operation";

    pub(crate) fn with_context(context: impl Display, message: impl Display) -> String {
        format!("{context}: {message}")
    }

    pub(crate) fn length_mismatch(expected: impl Display, actual: impl Display) -> String {
        format!("expected {expected} bytes, got {actual}")
    }

    pub(crate) fn truncated_field(
        field: impl Display,
        length: impl Display,
        remaining: impl Display,
    ) -> String {
        format!("truncated {field}: expected {length} bytes, {remaining} remain")
    }

    pub(crate) fn trailing_bytes(remaining: impl Display) -> String {
        format!("{remaining} trailing bytes")
    }

    pub(crate) fn unsupported_version(version: impl Display) -> String {
        format!("unsupported version {version}")
    }

    pub(crate) fn invalid_json_payload(error: impl Display) -> String {
        format!("invalid JSON payload: {error}")
    }
}

pub(crate) mod value {
    use std::fmt::Display;

    pub(crate) const CONVERSION_PREFIX: &str = "value conversion failed: ";

    pub(crate) const ENCODE: &str = "encode";

    pub(crate) const DECODE: &str = "decode";

    pub(crate) fn conversion(direction: impl Display, error: impl Display) -> String {
        format!("{direction}: {error}")
    }
}

pub(crate) mod observability {
    use std::fmt::Display;

    pub(crate) const ALREADY_INITIALIZED: &str = "observability is already initialized";

    pub(crate) const SHUT_DOWN: &str =
        "observability was shut down and cannot be initialized again in this process";

    pub(crate) const PARTIALLY_INITIALIZED: &str = "observability was only partially initialized and cannot be initialized again in this process";

    pub(crate) const SUBSCRIBER_PREFIX: &str = "failed to install the tracing subscriber: ";

    pub(crate) const EXPORTER_PREFIX: &str = "telemetry exporter error: ";

    pub(crate) const RECORDER_PREFIX: &str = "failed to install the metrics recorder: ";

    pub(crate) const INVALID_CONFIG_PREFIX: &str = "invalid configuration: ";

    pub(crate) fn tracer_shutdown_failed(error: impl Display) -> String {
        format!("tracer provider shutdown failed: {error}")
    }

    pub(crate) fn runtime_creation_failed(error: impl Display) -> String {
        format!("failed to create the runtime: {error}")
    }

    pub(crate) fn metrics_buckets_failed(metric: impl Display, error: impl Display) -> String {
        format!("failed to configure buckets for {metric}: {error}")
    }

    pub(crate) fn prometheus_exporter_failed(
        listen_addr: impl Display,
        error: impl Display,
    ) -> String {
        format!("failed to build the Prometheus exporter on {listen_addr}: {error}")
    }

    pub(crate) fn invalid_log_filter(filter: impl Display, error: impl Display) -> String {
        format!("invalid log filter '{filter}': {error}")
    }

    pub(crate) fn invalid_metrics_address(address: impl Display, error: impl Display) -> String {
        format!("invalid metrics_listen_addr '{address}': {error}")
    }

    pub(crate) fn missing_metrics_address(address: impl Display) -> String {
        format!("metrics_listen_addr '{address}' resolved to no address")
    }

    pub(crate) fn otlp_exporter_failed(endpoint: impl Display, error: impl Display) -> String {
        format!("failed to build the OTLP exporter for '{endpoint}': {error}")
    }
}

pub(crate) mod ffi {
    use std::fmt::Display;

    pub(crate) const NULL_CLIENT: &str = "[E998] client is null";

    pub(crate) const NULL_DATATYPE: &str = "[E998] datatype is null";

    pub(crate) const NULL_DATA: &str = "[E998] data is null";

    pub(crate) const PANIC: &str = "[E999] Rust panic caught at the FFI boundary";

    pub(crate) fn null_argument(name: impl Display) -> String {
        format!("[E998] {name} is null")
    }

    pub(crate) fn invalid_utf8_argument(name: impl Display) -> String {
        format!("[E998] {name} is not valid UTF-8")
    }

    pub(crate) fn wrong_handle_type(name: impl Display) -> String {
        format!("[E998] handle is not a {name}")
    }

    pub(crate) fn foreign_callback_aborted(code: impl Display) -> String {
        format!("aborted by foreign callback (code {code})")
    }

    pub(crate) fn unknown_log_format(value: impl Display) -> String {
        format!("unknown log_format {value} (expected 0=off, 1=json, 2=text)")
    }
}
