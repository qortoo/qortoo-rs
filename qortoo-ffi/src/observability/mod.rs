//! Observability entry points: explicit, process-global initialization and shutdown.
//!
//! Building a `Client` never installs a subscriber, a recorder, or an exporter. The
//! application owns that decision — log level, service identity, and endpoints — and
//! expresses it once through `qortoo_observability_init`. Until then the core's
//! instrumentation still runs but goes nowhere, which is the safe no-op state.
//!
//! The pipelines themselves live in the core; this crate enables its `observability`
//! umbrella feature so both managed exporters are available. This module is the C ABI
//! adapter over them: it converts the C options into
//! [`qortoo::ObservabilitySettings`] and maps failures onto FFI error codes.

use std::{ffi::c_char, time::Duration};

use qortoo::{
    LogFormat, MetricsSettings, ObservabilityError, ObservabilitySettings, TraceSettings,
    resolve_log_filter, resolve_metrics_listen_addr, resolve_otlp_endpoint,
};

use crate::{
    error::{QortooError, clear_err, observability_error_code, set_err},
    util::{cstr_arg, ffi_guard},
};

pub(crate) mod trace_context;

pub(crate) use trace_context::with_remote_parent;

/// Timeout applied by `qortoo_observability_shutdown` when the caller passes 0.
const DEFAULT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// Observability configuration. Every string is nullable; a null selects the default
/// described on the field. A zeroed struct installs nothing but is still valid.
#[repr(C)]
pub struct QortooObservabilityOptions {
    /// `service.name` reported to the trace backend. Null uses `"qortoo"`; set it to the
    /// same value the application uses for its own telemetry.
    pub service_name: *const c_char,
    /// Language of the binding, exported as `qortoo.binding.language` (e.g. `"go"`).
    /// Null omits the attribute.
    pub binding_language: *const c_char,
    /// `tracing` filter directives (e.g. `"qortoo=debug"`). Null falls back to `RUST_LOG`,
    /// then to `"qortoo=info"`.
    pub log_filter: *const c_char,
    /// Stdout log format: 0 = off, 1 = JSON, 2 = Qortoo's compact text format.
    /// Text colors are enabled only when stdout is connected to a terminal.
    pub log_format: i32,
    /// Exports spans over OTLP/gRPC.
    pub trace_enabled: bool,
    /// OTLP endpoint. Null falls back to `OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`, then
    /// `OTEL_EXPORTER_OTLP_ENDPOINT`, then `http://localhost:4317`.
    pub otlp_endpoint: *const c_char,
    /// Installs the `metrics` recorder and serves a Prometheus scrape endpoint.
    pub metrics_enabled: bool,
    /// Address of that endpoint. Null uses `0.0.0.0:9000`.
    pub metrics_listen_addr: *const c_char,
}

/// Installs the logging, tracing, and metrics pipelines described by `options`
/// (null selects every default). Call it once per process, before creating clients.
///
/// Fails with a distinct code when it is called twice, when it is called after
/// `qortoo_observability_shutdown`, when another library already owns the global
/// subscriber or recorder, when an exporter cannot start, or when the options are
/// invalid — the application must not silently lose the telemetry it configured.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_observability_init(
    options: *const QortooObservabilityOptions,
    err_out: *mut QortooError,
) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let Some(settings) = build_settings(options, err_out) else {
                return;
            };
            if let Err(e) = qortoo::init_observability(settings) {
                set_err(err_out, observability_error_code(&e), &e.to_string());
            }
        })
    }
}

/// Flushes pending telemetry and stops the exporters, waiting at most `timeout_ms`
/// (0 selects 5s). A no-op when nothing was initialized, so it is always safe to defer.
///
/// This is terminal: `tracing` allows one global subscriber per process, so a later
/// `qortoo_observability_init` fails instead of pretending to restore the pipelines.
/// Call it after every `qortoo_client_free`, and before the application shuts its own
/// telemetry down.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn qortoo_observability_shutdown(timeout_ms: u64, err_out: *mut QortooError) {
    unsafe {
        clear_err(err_out);
        ffi_guard(err_out, (), || {
            let timeout = match timeout_ms {
                0 => DEFAULT_SHUTDOWN_TIMEOUT,
                ms => Duration::from_millis(ms),
            };
            if let Err(e) = qortoo::shutdown_observability(timeout) {
                set_err(err_out, observability_error_code(&e), &e.to_string());
            }
        })
    }
}

/// Decodes the C encoding of [`LogFormat`]. The numbering is an ABI detail, so it lives
/// here rather than in the core.
fn log_format_from_i32(value: i32) -> Result<LogFormat, ObservabilityError> {
    match value {
        0 => Ok(LogFormat::Off),
        1 => Ok(LogFormat::Json),
        2 => Ok(LogFormat::Text),
        other => Err(ObservabilityError::InvalidConfig(format!(
            "unknown log_format {other} (expected 0=off, 1=json, 2=text)"
        ))),
    }
}

/// Converts the C options into resolved [`ObservabilitySettings`], reporting
/// configuration problems through `err_out`. Returns `None` when the options were
/// rejected.
unsafe fn build_settings(
    options: *const QortooObservabilityOptions,
    err_out: *mut QortooError,
) -> Option<ObservabilitySettings> {
    let options = unsafe { options.as_ref() };

    macro_rules! optional_arg {
        ($field:ident) => {
            match options.map(|o| o.$field) {
                Some(p) if !p.is_null() => {
                    Some(unsafe { cstr_arg(p, stringify!($field), err_out) }?)
                }
                _ => None,
            }
        };
    }

    let service_name = optional_arg!(service_name);
    let binding_language = optional_arg!(binding_language);
    let log_filter = optional_arg!(log_filter);
    let otlp_endpoint = optional_arg!(otlp_endpoint);
    let metrics_listen_addr = optional_arg!(metrics_listen_addr);

    let fail = |e: ObservabilityError| {
        unsafe { set_err(err_out, observability_error_code(&e), &e.to_string()) };
        None::<ObservabilitySettings>
    };

    let log_format = match log_format_from_i32(options.map_or(0, |o| o.log_format)) {
        Ok(format) => format,
        Err(e) => return fail(e),
    };

    let log_filter = match resolve_log_filter(log_filter) {
        Ok(filter) => filter,
        Err(e) => return fail(e),
    };

    let trace = options
        .is_some_and(|o| o.trace_enabled)
        .then(|| TraceSettings {
            endpoint: resolve_otlp_endpoint(otlp_endpoint),
        });

    let metrics = match options.is_some_and(|o| o.metrics_enabled) {
        true => match resolve_metrics_listen_addr(metrics_listen_addr) {
            Ok(listen_addr) => Some(MetricsSettings { listen_addr }),
            Err(e) => return fail(e),
        },
        false => None,
    };

    Some(ObservabilitySettings {
        service_name: service_name.unwrap_or_else(|| qortoo::DEFAULT_SERVICE_NAME.to_string()),
        binding_language,
        log_filter,
        log_format,
        trace,
        metrics,
    })
}

#[cfg(test)]
mod tests_observability {
    use super::*;

    #[test]
    fn can_decode_the_c_log_format_encoding() {
        assert_eq!(log_format_from_i32(0).unwrap(), LogFormat::Off);
        assert_eq!(log_format_from_i32(1).unwrap(), LogFormat::Json);
        assert_eq!(log_format_from_i32(2).unwrap(), LogFormat::Text);
        assert!(matches!(
            log_format_from_i32(7),
            Err(ObservabilityError::InvalidConfig(_))
        ));
    }
}
