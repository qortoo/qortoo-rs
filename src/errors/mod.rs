pub mod clients;
pub mod connectivity;
pub mod datatypes;
pub mod push_pull;

pub type BoxedError = Box<dyn std::error::Error + Send + Sync>;

pub fn with_stack_trace(
    trace: &std::backtrace::Backtrace,
    caller: &std::panic::Location,
) -> String {
    let bt_parsed = btparse::deserialize(trace).unwrap();
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

pub(crate) use with_err_out;

#[cfg(test)]
mod tests_datatype_errors {
    use std::io::{Error, ErrorKind};

    use crossbeam_channel::TrySendError;

    use crate::{ClientError, DatatypeError, datatypes::event_loop::Event, errors::with_err_out};

    #[test]
    fn can_assert_error() {
        fn assert_error<E: 'static + std::error::Error>() {}
        assert_error::<DatatypeError>();
        assert_error::<ClientError>();
    }

    #[test]
    fn can_compare_errors() {
        let source_err1 = Box::new(Error::new(ErrorKind::InvalidData, "source err1"));
        let source_err2 = Box::new(Error::new(ErrorKind::InvalidInput, "source err2"));
        let e1 = DatatypeError::FailedTransaction(source_err1);
        let e2 = DatatypeError::FailedTransaction(source_err2);
        assert_eq!(e1, e2);

        let e3 = DatatypeError::FailedToDeserialize("e2".to_string());
        assert_ne!(e2, e3);
    }

    #[test]
    fn can_use_err_macro() {
        let d1 = with_err_out!(DatatypeError::FailedToDeserialize(
            "datatype error".to_owned()
        ));
        let source_err1 = Box::new(Error::new(ErrorKind::InvalidInput, "source err1"));
        let d2 = with_err_out!(DatatypeError::FailedTransaction(source_err1));
        assert_ne!(d1, d2);
        let c1 = with_err_out!(ClientError::FailedToSubscribeOrCreateDatatype(
            "clients error".to_owned()
        ));
        let c2 = with_err_out!(ClientError::FailedToSubscribeOrCreateDatatype("".into()));
        assert_eq!(c1, c2);
        into_next_stack();
    }

    fn into_next_stack() {
        let err = Box::new(TrySendError::Full(Event::PushTransaction));
        let _d3 = with_err_out!(DatatypeError::FailureInEventLoop(err));
    }
}
