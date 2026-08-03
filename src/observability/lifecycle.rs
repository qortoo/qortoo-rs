//! Process-global observability lifecycle: install once, shut down once.
//!
//! Building a `Client` never reaches this module. The application decides — once, and
//! explicitly — whether Qortoo installs a `tracing` subscriber, a `metrics` recorder,
//! and their exporters. Until then the SDK's instrumentation still runs but goes
//! nowhere, which is the safe no-op state.
//!
//! The exporters live on a dedicated tokio runtime so their lifetime is independent of
//! any `Client` and its runtime: the last `Client` being dropped must not silently stop
//! the telemetry the application configured.

use std::{sync::Mutex, time::Duration};

#[cfg(feature = "observability-trace")]
use opentelemetry_sdk::trace::SdkTracerProvider;
use tokio::runtime::Runtime;

#[cfg(feature = "observability-metrics")]
use crate::observability::prometheus;
#[cfg(feature = "observability-trace")]
use crate::observability::subscriber;
use crate::{
    errors::observability::ObservabilityError, observability::settings::ObservabilitySettings,
};

/// Time budget for the rollback path, where nothing has been exported yet.
const ROLLBACK_TIMEOUT: Duration = Duration::from_secs(1);

/// Global state machine. `ShutDown` and `Failed` are terminal because process-global
/// subscribers and recorders cannot be removed after they have been installed.
enum Lifecycle {
    Uninitialized,
    Initialized(Box<Installed>),
    ShutDown,
    Failed,
}

/// An installation error plus whether an irreversible process-global was installed
/// before the error occurred.
struct InstallFailure {
    error: ObservabilityError,
    global_state_changed: bool,
}

impl InstallFailure {
    fn retriable(error: ObservabilityError) -> Self {
        Self {
            error,
            global_state_changed: false,
        }
    }
}

/// What an initialization owns and must release again.
struct Installed {
    /// Dedicated runtime for the exporters. The tonic OTLP channel binds to the runtime
    /// that is current when it is built, so this must outlive every export.
    ///
    /// Deliberately owned here instead of borrowed from
    /// [`crate::utils::runtime::get_or_init_runtime_handle`]: shutdown needs to drain the
    /// batch queue within a caller-supplied timeout, which a shared runtime cannot offer.
    runtime: Option<Runtime>,
    #[cfg(feature = "observability-trace")]
    tracer_provider: Option<SdkTracerProvider>,
}

impl Installed {
    /// Flushes and stops the exporters. The provider goes first: it needs the runtime
    /// alive to drain its batch queue.
    fn shutdown(self, timeout: Duration) -> Result<(), ObservabilityError> {
        #[cfg(not(feature = "observability-trace"))]
        let result = Ok(());
        #[cfg(feature = "observability-trace")]
        let mut result = Ok(());
        #[cfg(feature = "observability-trace")]
        if let Some(provider) = self.tracer_provider {
            if let Err(e) = provider.shutdown_with_timeout(timeout) {
                result = Err(ObservabilityError::Exporter(format!(
                    "tracer provider shutdown failed: {e}"
                )));
            }
        }
        if let Some(runtime) = self.runtime {
            runtime.shutdown_timeout(timeout);
        }
        result
    }
}

static STATE: Mutex<Lifecycle> = Mutex::new(Lifecycle::Uninitialized);

fn lock() -> std::sync::MutexGuard<'static, Lifecycle> {
    // A panic while holding the lock leaves the state readable and consistent (the
    // installers either finished or returned an error), so poisoning is not fatal here.
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Installs the logging, tracing, and metrics pipelines described by `settings`.
///
/// Call it once per process, before creating clients. It fails — rather than silently
/// doing nothing — when it is called twice, when it is called after
/// [`shutdown`], when another library already owns the global subscriber or
/// recorder, or when an exporter cannot start: an application must not lose the
/// telemetry it explicitly configured.
///
/// ```no_run
/// use qortoo::{LogFormat, ObservabilitySettings};
///
/// qortoo::init_observability(ObservabilitySettings {
///     service_name: "my-service".to_string(),
///     log_format: LogFormat::Json,
///     ..Default::default()
/// })?;
/// # Ok::<(), qortoo::ObservabilityError>(())
/// ```
pub fn init(settings: ObservabilitySettings) -> Result<(), ObservabilityError> {
    let mut state = lock();
    match &*state {
        Lifecycle::Initialized(_) => return Err(ObservabilityError::AlreadyInitialized),
        Lifecycle::ShutDown => return Err(ObservabilityError::ShutDown),
        Lifecycle::Failed => return Err(ObservabilityError::PartiallyInitialized),
        Lifecycle::Uninitialized => {}
    }
    finish_installation(&mut state, install(settings))
}

fn finish_installation(
    state: &mut Lifecycle,
    result: Result<Installed, InstallFailure>,
) -> Result<(), ObservabilityError> {
    match result {
        Ok(installed) => {
            *state = Lifecycle::Initialized(Box::new(installed));
            Ok(())
        }
        Err(failure) => {
            if failure.global_state_changed {
                *state = Lifecycle::Failed;
            }
            Err(failure.error)
        }
    }
}

fn install(settings: ObservabilitySettings) -> Result<Installed, InstallFailure> {
    let needs_runtime = needs_exporter_runtime(&settings);
    let runtime = match needs_runtime {
        true => Some(new_runtime().map_err(InstallFailure::retriable)?),
        false => None,
    };
    #[cfg(feature = "observability-trace")]
    let mut installed = Installed {
        runtime,
        tracer_provider: None,
    };
    #[cfg(not(feature = "observability-trace"))]
    let installed = Installed { runtime };
    #[cfg(not(feature = "observability-trace"))]
    let global_state_changed = false;
    #[cfg(feature = "observability-trace")]
    let mut global_state_changed = false;

    // Fallible builders run before their matching global installation. If a later
    // installation fails after the subscriber was installed, rollback can release
    // exporters but cannot remove that subscriber; the lifecycle then becomes terminal.
    let result = (|| {
        #[cfg(feature = "observability-trace")]
        if settings.installs_subscriber() {
            let provider = match &settings.trace {
                Some(trace) => {
                    let handle = installed
                        .runtime
                        .as_ref()
                        .expect("a trace exporter always creates the runtime")
                        .handle();
                    Some(subscriber::build_tracer_provider(&settings, trace, handle)?)
                }
                None => None,
            };
            subscriber::install(&settings, provider.as_ref())?;
            global_state_changed = true;
            installed.tracer_provider = provider;
        }
        #[cfg(feature = "observability-metrics")]
        if let Some(metrics_settings) = &settings.metrics {
            let handle = installed
                .runtime
                .as_ref()
                .expect("a metrics exporter always creates the runtime")
                .handle();
            prometheus::install(metrics_settings, handle)?;
        }
        Ok(())
    })();

    match result {
        Ok(()) => Ok(installed),
        Err(e) => {
            let _ = installed.shutdown(ROLLBACK_TIMEOUT);
            Err(InstallFailure {
                error: e,
                global_state_changed,
            })
        }
    }
}

fn needs_exporter_runtime(settings: &ObservabilitySettings) -> bool {
    let mut needed = false;
    #[cfg(feature = "observability-trace")]
    {
        needed |= settings.trace.is_some();
    }
    #[cfg(feature = "observability-metrics")]
    {
        needed |= settings.metrics.is_some();
    }
    needed
}

fn new_runtime() -> Result<Runtime, ObservabilityError> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .thread_name("qortoo-observability")
        .build()
        .map_err(|e| ObservabilityError::Exporter(format!("failed to create the runtime: {e}")))
}

/// Flushes pending telemetry and stops the exporters, waiting at most `timeout`.
///
/// A no-op (success) when nothing was initialized, so a deferred shutdown in the
/// application is always safe to call. This is terminal: a later [`init`] fails instead
/// of pretending to restore the pipelines.
pub fn shutdown(timeout: Duration) -> Result<(), ObservabilityError> {
    let mut state = lock();
    if !matches!(&*state, Lifecycle::Initialized(_)) {
        return Ok(());
    }
    match std::mem::replace(&mut *state, Lifecycle::ShutDown) {
        Lifecycle::Initialized(installed) => installed.shutdown(timeout),
        // Unreachable: the state was checked above while holding the lock.
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests_state {
    use super::*;

    #[test]
    fn can_treat_a_shutdown_without_init_as_a_no_op() {
        // The process-global state is shared with every other test in this binary, and
        // the test harness already installed a subscriber, so the only transition that
        // can be asserted here without side effects is the uninitialized one.
        assert!(shutdown(Duration::from_millis(10)).is_ok());
    }

    #[test]
    fn can_release_an_installation_that_owns_nothing() {
        let installed = Installed {
            runtime: None,
            #[cfg(feature = "observability-trace")]
            tracer_provider: None,
        };
        assert!(installed.shutdown(ROLLBACK_TIMEOUT).is_ok());
    }

    #[test]
    fn can_build_the_dedicated_exporter_runtime() {
        let runtime = new_runtime().expect("the exporter runtime must be creatable");
        runtime.shutdown_timeout(ROLLBACK_TIMEOUT);
    }

    #[test]
    fn can_distinguish_retriable_and_terminal_installation_failures() {
        let mut retriable_state = Lifecycle::Uninitialized;
        let retriable = InstallFailure::retriable(ObservabilityError::InvalidConfig("bad".into()));
        let result = finish_installation(&mut retriable_state, Err(retriable));
        assert!(matches!(result, Err(ObservabilityError::InvalidConfig(_))));
        assert!(matches!(retriable_state, Lifecycle::Uninitialized));

        let mut terminal_state = Lifecycle::Uninitialized;
        let terminal = InstallFailure {
            error: ObservabilityError::Recorder("already installed".into()),
            global_state_changed: true,
        };
        let result = finish_installation(&mut terminal_state, Err(terminal));
        assert!(matches!(result, Err(ObservabilityError::Recorder(_))));
        assert!(matches!(terminal_state, Lifecycle::Failed));
    }
}
