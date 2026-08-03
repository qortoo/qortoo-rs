package qortoo

/*
#include <stdlib.h>
#include <qortoo.h>
*/
import "C"

import (
	"context"
	"time"

	"go.opentelemetry.io/otel/propagation"
)

// The Rust core is instrumented with tracing spans and metrics, but a Go process has
// no way to install a Rust subscriber or recorder — so this package does it, once,
// when the application asks for it. Creating a client never starts an exporter.
//
// Until InitObservability is called the core's instrumentation runs and goes nowhere,
// which is the safe no-op state: RUST_LOG alone produces no output.

// LogFormat selects the stdout log stream written by the Rust side.
type LogFormat int32

const (
	// LogFormatOff writes no logs. Combined with TraceEnabled=false it leaves the
	// Rust global subscriber untouched for other libraries in the process.
	LogFormatOff LogFormat = 0
	// LogFormatJSON writes one structured JSON object per event.
	LogFormatJSON LogFormat = 1
	// LogFormatText writes Qortoo's compact human-readable format. Colors are enabled
	// only when stdout is connected to a terminal.
	LogFormatText LogFormat = 2
)

// ObservabilityOptions configures the SDK's telemetry pipelines. The zero value is
// valid and installs nothing.
type ObservabilityOptions struct {
	// ServiceName is reported as service.name to the trace backend. Empty uses
	// "qortoo"; set it to the same name the application uses for its own telemetry so
	// Go and Rust spans land in one service.
	ServiceName string
	// LogFilter holds tracing filter directives (e.g. "qortoo=debug"). Empty falls
	// back to RUST_LOG, then to "qortoo=info".
	LogFilter string
	// LogFormat selects the stdout log stream (default: none).
	LogFormat LogFormat

	// TraceEnabled exports spans over OTLP/gRPC.
	TraceEnabled bool
	// OTLPEndpoint overrides the trace endpoint. Empty falls back to
	// OTEL_EXPORTER_OTLP_TRACES_ENDPOINT, then OTEL_EXPORTER_OTLP_ENDPOINT, then
	// http://localhost:4317.
	OTLPEndpoint string

	// MetricsEnabled installs the metrics recorder and serves a Prometheus scrape
	// endpoint.
	MetricsEnabled bool
	// MetricsListenAddr is the address of that endpoint. Empty uses 0.0.0.0:9000.
	MetricsListenAddr string
}

// bindingLanguage is exported as the qortoo.binding.language resource attribute so
// spans coming through this package are distinguishable from pure-Rust ones.
const bindingLanguage = "go"

// InitObservability installs the SDK's logging, tracing, and metrics pipelines.
//
// Call it once per process, before creating clients. It returns an error when it is
// called twice, when it is called after ShutdownObservability, when another library
// already installed a Rust global subscriber or metrics recorder, when an exporter
// cannot start, or when the options are invalid — never silently, because losing
// configured telemetry is worse than failing loudly at startup.
func InitObservability(opts ObservabilityOptions) error {
	cOpts, free := opts.toC()
	defer free()

	var cerr C.QortooError
	C.qortoo_observability_init(cOpts, &cerr)
	return takeError(&cerr)
}

// ShutdownObservability flushes pending telemetry and stops the exporters, using the
// deadline remaining in ctx as its shutdown budget (5s when it has none). A context
// already canceled or expired is returned before crossing the FFI boundary. It is a
// no-op when nothing was initialized, so deferring it is always safe.
//
// This is terminal: a later InitObservability fails instead of pretending to restore
// the pipelines. The recommended order at exit is Counter.Close → Client.Close →
// ShutdownObservability → the application's own telemetry shutdown.
func ShutdownObservability(ctx context.Context) error {
	timeoutMS, err := shutdownTimeoutMSAt(ctx, time.Now())
	if err != nil {
		return err
	}

	var cerr C.QortooError
	C.qortoo_observability_shutdown(C.uint64_t(timeoutMS), &cerr)
	return takeError(&cerr)
}

// shutdownTimeoutMSAt converts the context deadline into the millisecond budget used
// by the C ABI. Positive sub-millisecond budgets round up to one: zero is reserved by
// the ABI to select its five-second default.
func shutdownTimeoutMSAt(ctx context.Context, now time.Time) (uint64, error) {
	if err := ctx.Err(); err != nil {
		return 0, err
	}

	deadline, ok := ctx.Deadline()
	if !ok {
		return 0, nil
	}
	remaining := deadline.Sub(now)
	if remaining <= 0 {
		return 0, context.DeadlineExceeded
	}

	milliseconds := remaining / time.Millisecond
	if remaining%time.Millisecond != 0 {
		milliseconds++
	}
	return uint64(milliseconds), nil
}

// toC builds the C options together with a function releasing the strings it owns.
func (opts ObservabilityOptions) toC() (*C.QortooObservabilityOptions, func()) {
	var strings []*C.char
	optional := func(s string) *C.char {
		if s == "" {
			return nil
		}
		c := cString(s)
		strings = append(strings, c)
		return c
	}

	cOpts := &C.QortooObservabilityOptions{
		service_name:        optional(opts.ServiceName),
		binding_language:    optional(bindingLanguage),
		log_filter:          optional(opts.LogFilter),
		log_format:          C.int32_t(opts.LogFormat),
		trace_enabled:       C.bool(opts.TraceEnabled),
		otlp_endpoint:       optional(opts.OTLPEndpoint),
		metrics_enabled:     C.bool(opts.MetricsEnabled),
		metrics_listen_addr: optional(opts.MetricsListenAddr),
	}
	return cOpts, func() {
		for _, s := range strings {
			freeCString(s)
		}
	}
}

// traceContext extracts the W3C trace-context headers of the span in ctx.
//
// An OpenTelemetry context cannot cross cgo, so the context-aware methods pass these
// two strings instead and Rust restores them as a remote parent. Both are empty when
// ctx carries no span, which makes the Rust side start a trace of its own.
var traceContextPropagator = propagation.TraceContext{}

func traceContext(ctx context.Context) (traceparent, tracestate string) {
	if ctx == nil {
		return "", ""
	}
	carrier := propagation.MapCarrier{}
	traceContextPropagator.Inject(ctx, carrier)
	return carrier.Get("traceparent"), carrier.Get("tracestate")
}

// withTraceContext runs body with the C strings of ctx's trace context (nil when
// absent) and releases them afterwards.
func withTraceContext(ctx context.Context, body func(traceparent, tracestate *C.char)) {
	traceparent, tracestate := traceContext(ctx)
	var cTraceparent, cTracestate *C.char
	if traceparent != "" {
		cTraceparent = cString(traceparent)
		defer freeCString(cTraceparent)
	}
	if tracestate != "" {
		cTracestate = cString(tracestate)
		defer freeCString(cTracestate)
	}
	body(cTraceparent, cTracestate)
}
