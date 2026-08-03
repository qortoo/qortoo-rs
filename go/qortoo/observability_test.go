package qortoo

import (
	"context"
	"os"
	"os/exec"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
	"go.opentelemetry.io/otel/trace"
)

const (
	testTraceID                  = "4bf92f3577b34da6a3ce929d0e0e4736"
	testSpanID                   = "00f067aa0ba902b7"
	observabilitySubprocessEnv   = "QORTOO_OBSERVABILITY_TEST_SUBPROCESS"
	observabilityLifecycleCase   = "lifecycle"
	observabilityJSONLoggingCase = "json-logging"
)

// contextWithSpan builds a context carrying a sampled remote span, which is what an
// instrumented Go application hands to the context-aware methods.
func contextWithSpan(t *testing.T, tracestate string) context.Context {
	t.Helper()
	traceID, err := trace.TraceIDFromHex(testTraceID)
	require.NoError(t, err)
	spanID, err := trace.SpanIDFromHex(testSpanID)
	require.NoError(t, err)

	state := trace.TraceState{}
	if tracestate != "" {
		state, err = trace.ParseTraceState(tracestate)
		require.NoError(t, err)
	}
	return trace.ContextWithSpanContext(context.Background(), trace.NewSpanContext(trace.SpanContextConfig{
		TraceID:    traceID,
		SpanID:     spanID,
		TraceFlags: trace.FlagsSampled,
		TraceState: state,
		Remote:     true,
	}))
}

func TestTraceContextExtraction(t *testing.T) {
	traceparent, tracestate := traceContext(contextWithSpan(t, "vendor=value"))
	require.Equal(t, "00-"+testTraceID+"-"+testSpanID+"-01", traceparent)
	require.Equal(t, "vendor=value", tracestate)
}

func TestTraceContextWithoutSpanIsEmpty(t *testing.T) {
	traceparent, tracestate := traceContext(context.Background())
	require.Empty(t, traceparent)
	require.Empty(t, tracestate)

	// A nil context must not panic either: it reaches us from callers we do not control.
	var missing context.Context
	traceparent, tracestate = traceContext(missing)
	require.Empty(t, traceparent)
	require.Empty(t, tracestate)
}

// The context-aware methods must behave exactly like their plain counterparts when
// no observability pipeline is installed — that is the default state of a process.
func TestContextAwareOperationsWithoutObservability(t *testing.T) {
	conn := NewLocalConnectivity()
	defer conn.Close()
	conn.SetRealtime(false)

	client, err := NewClient("go-binding-obs", "ctx-ops", WithLocalConnectivity(conn))
	require.NoError(t, err)
	defer client.Close()

	counter, err := client.CreateCounter("ctx-counter", nil)
	require.NoError(t, err)
	defer counter.Close()

	ctx := contextWithSpan(t, "")

	require.NoError(t, counter.TransactionContext(ctx, "tx", func(tx *Counter) error {
		_, err := tx.IncreaseBy(3)
		return err
	}))
	require.Equal(t, int64(3), counter.Value())

	require.NoError(t, counter.SyncContext(ctx))
	require.Equal(t, StateSubscribed, counter.State())

	// A context without a span takes the same path with empty headers.
	require.NoError(t, counter.SyncContext(context.Background()))
}

func TestInitObservabilityRejectsInvalidOptions(t *testing.T) {
	tests := []struct {
		name string
		opts ObservabilityOptions
	}{
		{
			name: "unknown log format",
			opts: ObservabilityOptions{LogFormat: LogFormat(42)},
		},
		{
			name: "invalid log filter",
			opts: ObservabilityOptions{LogFormat: LogFormatText, LogFilter: "=nonsense="},
		},
		{
			name: "invalid metrics address",
			opts: ObservabilityOptions{
				MetricsEnabled:    true,
				MetricsListenAddr: "definitely not an address",
			},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			err := InitObservability(tt.opts)
			require.ErrorIs(t, err, &Error{Code: ErrCodeObservabilityInvalidConfig})
			require.NotEmpty(t, err.Error())
		})
	}
}

func TestObservabilityLifecycle(t *testing.T) {
	runObservabilitySubprocess(t, observabilityLifecycleCase)
}

func TestObservabilityJSONLogging(t *testing.T) {
	output := runObservabilitySubprocess(t, observabilityJSONLoggingCase)
	require.Contains(t, output, `"level":"TRACE"`)
	require.Contains(t, output, `"target":"qortoo::datatypes::counter"`)
	require.Contains(t, output, `"message":"increased by 1`)
}

func TestShutdownObservabilityRejectsExpiredContextBeforeFFI(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 0)
	defer cancel()
	require.ErrorIs(t, ShutdownObservability(ctx), context.DeadlineExceeded)
}

func TestShutdownTimeoutConversion(t *testing.T) {
	t.Run("no deadline selects the FFI default", func(t *testing.T) {
		timeout, err := shutdownTimeoutMSAt(context.Background(), time.Now())
		require.NoError(t, err)
		require.Zero(t, timeout)
	})

	t.Run("a canceled context is rejected without a deadline", func(t *testing.T) {
		ctx, cancel := context.WithCancel(context.Background())
		cancel()
		_, err := shutdownTimeoutMSAt(ctx, time.Now())
		require.ErrorIs(t, err, context.Canceled)
	})

	t.Run("an expired deadline is rejected", func(t *testing.T) {
		deadline := time.Now().Add(-time.Second)
		ctx, cancel := context.WithDeadline(context.Background(), deadline)
		defer cancel()
		_, err := shutdownTimeoutMSAt(ctx, deadline.Add(time.Second))
		require.ErrorIs(t, err, context.DeadlineExceeded)
	})

	t.Run("a positive sub-millisecond budget never becomes the FFI default", func(t *testing.T) {
		deadline := time.Now().Add(time.Hour)
		ctx, cancel := context.WithDeadline(context.Background(), deadline)
		defer cancel()
		timeout, err := shutdownTimeoutMSAt(ctx, deadline.Add(-time.Nanosecond))
		require.NoError(t, err)
		require.Equal(t, uint64(1), timeout)
	})

	t.Run("a fractional millisecond rounds up", func(t *testing.T) {
		deadline := time.Now().Add(time.Hour)
		ctx, cancel := context.WithDeadline(context.Background(), deadline)
		defer cancel()
		timeout, err := shutdownTimeoutMSAt(ctx, deadline.Add(-1500*time.Microsecond))
		require.NoError(t, err)
		require.Equal(t, uint64(2), timeout)
	})
}

// Each scenario runs in a fresh copy of the Go test binary because Rust's tracing
// subscriber, metrics recorder, and Qortoo lifecycle are process-global and terminal.
func runObservabilitySubprocess(t *testing.T, scenario string) string {
	t.Helper()
	cmd := exec.Command(os.Args[0], "-test.run=^TestObservabilitySubprocess$")
	cmd.Env = append(os.Environ(), observabilitySubprocessEnv+"="+scenario)
	output, err := cmd.CombinedOutput()
	require.NoErrorf(t, err, "observability subprocess failed:\n%s", output)
	return string(output)
}

func TestObservabilitySubprocess(t *testing.T) {
	switch os.Getenv(observabilitySubprocessEnv) {
	case "":
		return
	case observabilityLifecycleCase:
		exerciseObservabilityLifecycle(t)
	case observabilityJSONLoggingCase:
		exerciseJSONLogging(t)
	default:
		require.FailNow(t, "unknown observability subprocess scenario")
	}
}

func exerciseObservabilityLifecycle(t *testing.T) {
	opts := ObservabilityOptions{
		ServiceName: "qortoo-go-lifecycle-test",
		LogFilter:   "qortoo=info",
		LogFormat:   LogFormatJSON,
	}

	require.NoError(t, InitObservability(opts))
	require.ErrorIs(t, InitObservability(opts), &Error{Code: ErrCodeObservabilityAlreadyInitialized})

	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	require.NoError(t, ShutdownObservability(ctx))
	require.NoError(t, ShutdownObservability(ctx), "a repeated shutdown is a no-op")
	require.ErrorIs(t, InitObservability(opts), &Error{Code: ErrCodeObservabilityShutDown})
}

func exerciseJSONLogging(t *testing.T) {
	opts := ObservabilityOptions{
		ServiceName: "qortoo-go-json-log-test",
		LogFilter:   "qortoo=trace",
		LogFormat:   LogFormatJSON,
	}
	require.NoError(t, InitObservability(opts))

	client, err := NewClient("go-binding-observability", "json-log")
	require.NoError(t, err)
	counter, err := client.CreateCounter("json-log-counter", nil)
	require.NoError(t, err)
	_, err = counter.IncreaseBy(1)
	require.NoError(t, err)

	counter.Close()
	client.Close()
	require.NoError(t, ShutdownObservability(context.Background()))
}
