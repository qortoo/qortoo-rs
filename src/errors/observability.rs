//! Failure modes of the opt-in, process-global observability installation.
//!
//! Every variant is a reason the application's telemetry configuration was not fully
//! applied. Installation reports rather than swallows these: a process that asked for
//! JSON logs and OTLP export must not silently run without them.

use thiserror::Error;

use crate::errors::messages::observability as msg;

/// Why [`crate::init_observability`] or [`crate::shutdown_observability`] failed.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ObservabilityError {
    /// `init_observability` was already called in this process.
    #[error("[E{code}] {}", msg::ALREADY_INITIALIZED, code = self.code())]
    AlreadyInitialized,

    /// `init_observability` was called after `shutdown_observability`; the installed
    /// process-global subscriber or recorder cannot be removed and restored reliably.
    #[error("[E{code}] {}", msg::SHUT_DOWN, code = self.code())]
    ShutDown,

    /// A previous initialization installed an irreversible process-global component
    /// before a later component failed.
    #[error("[E{code}] {}", msg::PARTIALLY_INITIALIZED, code = self.code())]
    PartiallyInitialized,

    /// The global `tracing` subscriber was already installed by someone else.
    #[error("[E{code}] {prefix}{0}", code = self.code(), prefix = msg::SUBSCRIBER_PREFIX)]
    Subscriber(String),

    /// Building, flushing, or stopping a telemetry exporter failed.
    #[error("[E{code}] {prefix}{0}", code = self.code(), prefix = msg::EXPORTER_PREFIX)]
    Exporter(String),

    /// The global `metrics` recorder was already installed by someone else.
    #[error("[E{code}] {prefix}{0}", code = self.code(), prefix = msg::RECORDER_PREFIX)]
    Recorder(String),

    /// The settings do not describe a valid configuration.
    #[error("[E{code}] {prefix}{0}", code = self.code(), prefix = msg::INVALID_CONFIG_PREFIX)]
    InvalidConfig(String),
}

impl ObservabilityError {
    /// Returns the stable numeric diagnostic code shared with language bindings.
    ///
    /// Use this value for programmatic decisions rather than parsing the message.
    pub const fn code(&self) -> i32 {
        match self {
            Self::AlreadyInitialized => 900,
            Self::ShutDown => 901,
            Self::Subscriber(_) => 902,
            Self::Exporter(_) => 903,
            Self::Recorder(_) => 904,
            Self::InvalidConfig(_) => 905,
            Self::PartiallyInitialized => 906,
        }
    }
}

#[cfg(test)]
mod tests_observability_errors {
    use super::*;

    #[test]
    fn can_describe_every_variant_distinctly() {
        let messages = [
            ObservabilityError::AlreadyInitialized.to_string(),
            ObservabilityError::ShutDown.to_string(),
            ObservabilityError::PartiallyInitialized.to_string(),
            ObservabilityError::Subscriber("s".into()).to_string(),
            ObservabilityError::Exporter("e".into()).to_string(),
            ObservabilityError::Recorder("r".into()).to_string(),
            ObservabilityError::InvalidConfig("c".into()).to_string(),
        ];
        let mut unique = messages.to_vec();
        unique.sort();
        unique.dedup();
        assert_eq!(
            unique.len(),
            messages.len(),
            "each variant must be distinguishable in a log line"
        );
    }
}
