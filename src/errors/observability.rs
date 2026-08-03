//! Failure modes of the opt-in, process-global observability installation.
//!
//! Every variant is a reason the application's telemetry configuration was not fully
//! applied. Installation reports rather than swallows these: a process that asked for
//! JSON logs and OTLP export must not silently run without them.

use thiserror::Error;

/// Why [`crate::init_observability`] or [`crate::shutdown_observability`] failed.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ObservabilityError {
    /// `init_observability` was already called in this process.
    #[error("observability is already initialized")]
    AlreadyInitialized,

    /// `init_observability` was called after `shutdown_observability`; the installed
    /// process-global subscriber or recorder cannot be removed and restored reliably.
    #[error("observability was shut down and cannot be initialized again in this process")]
    ShutDown,

    /// A previous initialization installed an irreversible process-global component
    /// before a later component failed.
    #[error(
        "observability was only partially initialized and cannot be initialized again in this process"
    )]
    PartiallyInitialized,

    /// The global `tracing` subscriber was already installed by someone else.
    #[error("failed to install the tracing subscriber: {0}")]
    Subscriber(String),

    /// Building, flushing, or stopping a telemetry exporter failed.
    #[error("telemetry exporter error: {0}")]
    Exporter(String),

    /// The global `metrics` recorder was already installed by someone else.
    #[error("failed to install the metrics recorder: {0}")]
    Recorder(String),

    /// The settings do not describe a valid configuration.
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
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
