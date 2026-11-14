pub mod clients;
pub mod connectivity;
pub mod datatypes;
pub(crate) mod messages;
#[cfg(any(feature = "observability-trace", feature = "observability-metrics"))]
pub mod observability;
pub mod push_pull;

/// A type alias for a boxed error that is thread-safe.
///
/// This is used throughout Qortoo to represent errors that can be passed
/// across thread boundaries. It wraps any error implementing the standard
/// `Error` trait along with `Send` and `Sync`.
///
/// # Examples
///
/// ```
/// use qortoo::BoxedError;
///
/// fn may_fail() -> Result<(), BoxedError> {
///     Err("something went wrong".into())
/// }
/// ```
pub type BoxedError = Box<dyn std::error::Error + Send + Sync>;

pub fn with_stack_trace(
    trace: &std::backtrace::Backtrace,
    caller: &std::panic::Location,
) -> String {
    // Stack traces are diagnostic only. Preserve the original error when a
    // platform produces an empty or otherwise unparsable backtrace.
    let Ok(bt_parsed) = btparse::deserialize(trace) else {
        return String::new();
    };
    let mut s = String::new();
    let mut stack_count = 0;
    for frame in bt_parsed.frames {
        if frame.function.contains(crate::constants::SDK_NAME) {
            stack_count += 1;
            let file_str = frame.file.unwrap_or_default();
            let mut file = file_str.strip_prefix("./").unwrap_or(&file_str).to_owned();
            let mut line = frame.line.unwrap_or_default();
            let space = " ".repeat(stack_count);
            if stack_count == 1 {
                file = caller.file().to_owned();
                line = caller.line() as usize;
            }

            std::fmt::Write::write_fmt(
                &mut s,
                format_args!("\n{}↘︎ {} 🗂️ {}:{}", space, frame.function, file, line),
            )
            .unwrap();
        }
    }
    s
}

macro_rules! with_err_out {
    ($err:expr) => {{
        let err = $err;
        let trace = std::backtrace::Backtrace::force_capture();
        let caller = std::panic::Location::caller();
        let s = crate::errors::with_stack_trace(&trace, &caller);
        tracing::error!("\x1b[31m{err}\x1b[0m{s}");
        err
    }};
}

#[cfg(test)]
macro_rules! equal_errors {
    ($err1:expr, $err2:expr) => {{ std::mem::discriminant($err1) == std::mem::discriminant($err2) }};
}

#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use equal_errors;
pub(crate) use with_err_out;

#[cfg(test)]
mod tests_datatype_errors {
    use std::{
        backtrace::Backtrace,
        io::{Error, ErrorKind},
        panic::Location,
    };

    use crossbeam_channel::TrySendError;

    use crate::{ClientError, DatatypeError, datatypes::event_loop::Event};

    #[test]
    fn can_assert_error() {
        fn assert_error<E: 'static + std::error::Error>() {}
        assert_error::<DatatypeError>();
        assert_error::<ClientError>();
    }

    #[test]
    fn can_preserve_unique_codes_and_matching_identifiers() {
        fn check(code: i32, expected: i32, text: String) -> i32 {
            assert_eq!(code, expected, "stable code changed: {text}");
            assert!(
                text.starts_with(&format!("[E{code}] ")),
                "identifier disagrees with code: {text}"
            );
            code
        }
        let codes = [
            {
                let error = ClientError::InvalidCollectionName(String::new());
                check(error.code(), 100, error.to_string())
            },
            {
                let error = ClientError::FailedToSubscribeOrCreateDatatype(String::new());
                check(error.code(), 101, error.to_string())
            },
            {
                let error = DatatypeError::TransactionFailed(String::new());
                check(error.code(), 201, error.to_string())
            },
            {
                let error = DatatypeError::Internal(String::new());
                check(error.code(), 202, error.to_string())
            },
            {
                let error = DatatypeError::Disallowed(String::new());
                check(error.code(), 205, error.to_string())
            },
            {
                let error = DatatypeError::NotWritable(String::new());
                check(error.code(), 206, error.to_string())
            },
            {
                let error = DatatypeError::ReadonlyViolation;
                check(error.code(), 207, error.to_string())
            },
            {
                let error = DatatypeError::SyncFailed(String::new());
                check(error.code(), 210, error.to_string())
            },
            {
                let error = DatatypeError::PushBufferExceededMaxMemSize;
                check(error.code(), 211, error.to_string())
            },
            {
                let error = DatatypeError::ServerRejected(crate::ServerRejectReason::CreateFailed(
                    String::new(),
                ));
                check(error.code(), 213, error.to_string())
            },
            {
                let error = DatatypeError::ValueConversion(String::new());
                check(error.code(), 214, error.to_string())
            },
            {
                let error = super::push_pull::PushPullError::ProtocolViolation(String::new());
                check(error.code(), 301, error.to_string())
            },
            {
                let error = super::push_pull::PushPullError::ReadonlyViolation;
                check(error.code(), 302, error.to_string())
            },
            {
                let error = super::push_pull::PushPullError::CreateFailed(String::new());
                check(error.code(), 303, error.to_string())
            },
            {
                let error = super::push_pull::PushPullError::ResourceNotFound(String::new());
                check(error.code(), 304, error.to_string())
            },
            {
                let error = super::push_pull::PushPullError::MissingSubscription(String::new());
                check(error.code(), 305, error.to_string())
            },
            {
                let error = super::push_pull::PushPullError::ServerInternalError(String::new());
                check(error.code(), 306, error.to_string())
            },
            #[cfg(any(feature = "observability-trace", feature = "observability-metrics"))]
            {
                let error = super::observability::ObservabilityError::AlreadyInitialized;
                check(error.code(), 900, error.to_string())
            },
            #[cfg(any(feature = "observability-trace", feature = "observability-metrics"))]
            {
                let error = super::observability::ObservabilityError::ShutDown;
                check(error.code(), 901, error.to_string())
            },
            #[cfg(any(feature = "observability-trace", feature = "observability-metrics"))]
            {
                let error = super::observability::ObservabilityError::Subscriber(String::new());
                check(error.code(), 902, error.to_string())
            },
            #[cfg(any(feature = "observability-trace", feature = "observability-metrics"))]
            {
                let error = super::observability::ObservabilityError::Exporter(String::new());
                check(error.code(), 903, error.to_string())
            },
            #[cfg(any(feature = "observability-trace", feature = "observability-metrics"))]
            {
                let error = super::observability::ObservabilityError::Recorder(String::new());
                check(error.code(), 904, error.to_string())
            },
            #[cfg(any(feature = "observability-trace", feature = "observability-metrics"))]
            {
                let error = super::observability::ObservabilityError::InvalidConfig(String::new());
                check(error.code(), 905, error.to_string())
            },
            #[cfg(any(feature = "observability-trace", feature = "observability-metrics"))]
            {
                let error = super::observability::ObservabilityError::PartiallyInitialized;
                check(error.code(), 906, error.to_string())
            },
        ];
        let mut unique = codes.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            codes.len(),
            "diagnostic codes must not collide"
        );
    }

    #[test]
    fn can_preserve_complete_error_messages() {
        let cases = [
            (
                super::clients::ClientError::InvalidCollectionName("detail".into()).to_string(),
                "[E100] invalid collection name: detail",
            ),
            (
                super::clients::ClientError::FailedToSubscribeOrCreateDatatype("detail".into())
                    .to_string(),
                "[E101] cannot subscribe or create datatype: detail",
            ),
            (
                super::datatypes::InternalReason::Deserialize("detail".into()).to_string(),
                "deserialize: detail",
            ),
            (
                super::datatypes::InternalReason::ExecuteOperation("detail".into()).to_string(),
                "execute operation: detail",
            ),
            (
                super::datatypes::InternalReason::EventLoop("detail".into()).to_string(),
                "event loop: detail",
            ),
            (
                super::datatypes::InternalReason::NonSequentialCseq.to_string(),
                "non-sequential cseq in push buffer",
            ),
            (
                super::datatypes::InternalReason::GetPushingTransactions.to_string(),
                "failed to get pushing transactions",
            ),
            (
                super::datatypes::DatatypeError::TransactionFailed("detail".into()).to_string(),
                "[E201] transaction failed: detail",
            ),
            (
                super::datatypes::DatatypeError::Internal("detail".into()).to_string(),
                "[E202] internal error: detail",
            ),
            (
                super::datatypes::DatatypeError::Disallowed("detail".into()).to_string(),
                "[E205] operation not allowed: detail",
            ),
            (
                super::datatypes::DatatypeError::NotWritable("detail".into()).to_string(),
                "[E206] not writable: detail",
            ),
            (
                super::datatypes::DatatypeError::ReadonlyViolation.to_string(),
                "[E207] readonly violation",
            ),
            (
                super::datatypes::DatatypeError::SyncFailed("detail".into()).to_string(),
                "[E210] sync failed: detail",
            ),
            (
                super::datatypes::DatatypeError::PushBufferExceededMaxMemSize.to_string(),
                "[E211] push buffer memory limit exceeded",
            ),
            (
                super::datatypes::DatatypeError::ServerRejected(
                    super::datatypes::ServerRejectReason::CreateFailed("detail".into()),
                )
                .to_string(),
                "[E213] server rejected: creation failed: detail",
            ),
            (
                super::datatypes::DatatypeError::ValueConversion("detail".into()).to_string(),
                "[E214] value conversion failed: detail",
            ),
            (
                super::connectivity::ConnectivityError::TimedOut("detail".into()).to_string(),
                "connection timed out: detail",
            ),
            (
                super::connectivity::ConnectivityError::TimedOut("detail".into())
                    .to_datatype_error()
                    .to_string(),
                "[E210] sync failed: connection timed out: detail",
            ),
            (
                super::push_pull::PushPullError::ProtocolViolation("detail".into()).to_string(),
                "[E301] protocol violation: detail",
            ),
            (
                super::push_pull::PushPullError::ReadonlyViolation.to_string(),
                "[E302] readonly client attempted write operation",
            ),
            (
                super::push_pull::PushPullError::CreateFailed("detail".into()).to_string(),
                "[E303] creation failed: detail",
            ),
            (
                super::push_pull::PushPullError::ResourceNotFound("detail".into()).to_string(),
                "[E304] resource not found: detail",
            ),
            (
                super::push_pull::PushPullError::MissingSubscription("detail".into()).to_string(),
                "[E305] missing subscription: detail",
            ),
            (
                super::push_pull::PushPullError::ServerInternalError("detail".into()).to_string(),
                "[E306] server internal error: detail",
            ),
        ];
        for (actual, expected) in cases {
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn can_describe_server_rejection_reasons() {
        use super::datatypes::{RecoveryAction, ServerRejectReason};

        let cases = [
            (
                ServerRejectReason::CreateFailed("datatype already exists".into()),
                "creation failed: datatype already exists",
                "[E213] server rejected: creation failed: datatype already exists",
                RecoveryAction::Disable,
            ),
            (
                ServerRejectReason::ResourceNotFound("Counter 'count' does not exist".into()),
                "resource not found: Counter 'count' does not exist",
                "[E213] server rejected: resource not found: Counter 'count' does not exist",
                RecoveryAction::Disable,
            ),
            (
                ServerRejectReason::MissingSubscription(
                    "client 'client-1' has no active subscription".into(),
                ),
                "missing subscription: client 'client-1' has no active subscription",
                "[E213] server rejected: missing subscription: client 'client-1' has no active subscription",
                RecoveryAction::Resubscribe,
            ),
            (
                ServerRejectReason::ProtocolViolation("unexpected \"state\"\n상세".into()),
                "protocol violation: unexpected \"state\"\n상세",
                "[E213] server rejected: protocol violation: unexpected \"state\"\n상세",
                RecoveryAction::Disable,
            ),
        ];
        for (reason, expected_reason, expected_error, recovery) in cases {
            assert_eq!(reason.to_string(), expected_reason);
            let error = DatatypeError::ServerRejected(reason);
            assert_eq!(error.code(), 213);
            assert_eq!(error.to_string(), expected_error);
            assert_eq!(error.mapping().recovery, recovery);
        }
    }

    #[test]
    fn can_describe_error_context() {
        use super::{
            datatypes::InternalReason,
            messages::{client as client_msg, datatype as datatype_msg, sync as sync_msg},
            push_pull::PushPullError,
        };
        use crate::{DataType, DatatypeState};

        let cases = [
            (
                DatatypeError::Disallowed(client_msg::datatype_not_managed("count")).to_string(),
                "[E205] operation not allowed: datatype 'count' is not managed by this client",
            ),
            (
                ClientError::FailedToSubscribeOrCreateDatatype(
                    client_msg::datatype_already_managed(
                        DataType::Variable,
                        "count",
                        DatatypeState::Creating,
                        DataType::Counter,
                    ),
                )
                .to_string(),
                "[E101] cannot subscribe or create datatype: cannot register Variable 'count' in Creating state: this client already manages the key as Counter",
            ),
            (
                DatatypeError::NotWritable(datatype_msg::not_writable(
                    "count",
                    DatatypeState::Subscribing,
                ))
                .to_string(),
                "[E206] not writable: cannot modify datatype 'count' in Subscribing state",
            ),
            (
                InternalReason::EventLoop(sync_msg::response_failed("channel closed"))
                    .into_error()
                    .to_string(),
                "[E202] internal error: event loop: failed to receive sync() response: channel closed",
            ),
            (
                PushPullError::CreateFailed(sync_msg::ALREADY_EXISTS.into()).to_string(),
                "[E303] creation failed: datatype already exists",
            ),
            (
                PushPullError::MissingSubscription(sync_msg::missing_active_subscription(
                    "client-1",
                ))
                .to_string(),
                "[E305] missing subscription: client 'client-1' has no active datatype subscription on this server",
            ),
            (
                PushPullError::ResourceNotFound(sync_msg::resource_not_found("Counter", "count"))
                    .to_string(),
                "[E304] resource not found: Counter 'count' does not exist",
            ),
            (
                PushPullError::ResourceNotFound(sync_msg::resource_type_mismatch(
                    "count", "Variable", "Counter",
                ))
                .to_string(),
                "[E304] resource not found: datatype type mismatch for 'count': requested Variable, existing Counter",
            ),
        ];
        for (actual, expected) in cases {
            assert_eq!(actual, expected);
        }
    }

    #[cfg(any(feature = "observability-trace", feature = "observability-metrics"))]
    #[test]
    fn can_preserve_observability_error_messages() {
        let cases = [
            (
                super::observability::ObservabilityError::AlreadyInitialized.to_string(),
                "[E900] observability is already initialized",
            ),
            (
                super::observability::ObservabilityError::ShutDown.to_string(),
                "[E901] observability was shut down and cannot be initialized again in this process",
            ),
            (
                super::observability::ObservabilityError::PartiallyInitialized.to_string(),
                "[E906] observability was only partially initialized and cannot be initialized again in this process",
            ),
            (
                super::observability::ObservabilityError::Subscriber("detail".into()).to_string(),
                "[E902] failed to install the tracing subscriber: detail",
            ),
            (
                super::observability::ObservabilityError::Exporter("detail".into()).to_string(),
                "[E903] telemetry exporter error: detail",
            ),
            (
                super::observability::ObservabilityError::Recorder("detail".into()).to_string(),
                "[E904] failed to install the metrics recorder: detail",
            ),
            (
                super::observability::ObservabilityError::InvalidConfig("detail".into())
                    .to_string(),
                "[E905] invalid configuration: detail",
            ),
        ];
        for (actual, expected) in cases {
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn can_compare_errors() {
        let source_err1 = Error::new(ErrorKind::InvalidData, "source err1");
        let source_err2 = Error::new(ErrorKind::InvalidInput, "source err2");
        let e1 = DatatypeError::TransactionFailed(source_err1.to_string());
        let e2 = DatatypeError::TransactionFailed(source_err2.to_string());
        assert_eq!(e1, e2); // same variant — payload is ignored by PartialEq
        let e3 = DatatypeError::Internal("e2".to_string());
        assert_ne!(e2, e3);
        assert!(equal_errors!(&e1, &e2));
        assert!(!equal_errors!(&e1, &e3));
        let e4 = e3.clone();
        assert_eq!(e3, e4);
    }

    #[test]
    fn can_omit_an_unavailable_stack_trace() {
        let stack_trace = super::with_stack_trace(&Backtrace::disabled(), Location::caller());

        assert!(stack_trace.is_empty());
    }

    #[test]
    fn can_use_err_macro() {
        let d1 = with_err_out!(DatatypeError::Internal("datatype error".to_owned()));
        let source_err1 = Error::new(ErrorKind::InvalidInput, "source err1");
        let d2 = with_err_out!(DatatypeError::TransactionFailed(source_err1.to_string()));
        assert_ne!(d1, d2);
        let c1 = with_err_out!(ClientError::FailedToSubscribeOrCreateDatatype(
            "clients error".into()
        ));
        let c2 = with_err_out!(ClientError::FailedToSubscribeOrCreateDatatype("".into()));
        assert_eq!(c1, c2);
        into_next_stack();
    }

    fn into_next_stack() {
        let err = TrySendError::Full(Event::PushTransaction {
            resp_tx: None,
            caller: tracing::Span::none(),
        });
        let _d3 = with_err_out!(DatatypeError::Internal(err.to_string()));
    }
}
