package qortoo

// White-box tests for handle lifetimes: the runtime.AddCleanup backstop, the
// exactly-once Close contract, and the failure-path userdata_drop contract.
//
// GC-driven cleanups run asynchronously, so positive assertions poll with
// require.Eventually while forcing collections; what these tests never do is
// assume a cleanup runs at one specific point.

import (
	"runtime"
	"sync"
	"testing"
	"time"

	"github.com/stretchr/testify/require"
)

type lifecycleRecorder struct {
	mu     sync.Mutex
	counts map[string]int
}

func (r *lifecycleRecorder) record(event string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.counts[event]++
}

func (r *lifecycleRecorder) count(event string) int {
	r.mu.Lock()
	defer r.mu.Unlock()
	return r.counts[event]
}

// drainCleanups flushes cleanups left over from earlier tests so they cannot
// pollute the counts of the recorder installed next.
func drainCleanups() {
	for range 3 {
		runtime.GC()
		time.Sleep(10 * time.Millisecond)
	}
}

func installLifecycleRecorder(t *testing.T) *lifecycleRecorder {
	t.Helper()
	drainCleanups()
	rec := &lifecycleRecorder{counts: map[string]int{}}
	hook := func(event string) { rec.record(event) }
	lifecycleHook.Store(&hook)
	t.Cleanup(func() { lifecycleHook.Store(nil) })
	return rec
}

// gcEventually polls cond while forcing garbage collections.
func gcEventually(t *testing.T, cond func() bool, msg string) {
	t.Helper()
	require.Eventually(t, func() bool {
		runtime.GC()
		return cond()
	}, 10*time.Second, 50*time.Millisecond, msg)
}

// gcSettle forces several collections and gives pending cleanups time to run,
// for negative assertions ("this cleanup must NOT have fired").
func gcSettle() {
	for range 5 {
		runtime.GC()
		time.Sleep(20 * time.Millisecond)
	}
}

func TestCleanupFreesUnreferencedHandles(t *testing.T) {
	rec := installLifecycleRecorder(t)

	func() {
		conn := NewLocalConnectivity()
		client, err := NewClient("go-lifecycle-test", "gc-all", WithLocalConnectivity(conn))
		require.NoError(t, err)
		counter, err := client.CreateCounter("gc-all-counter", nil)
		require.NoError(t, err)
		_, err = counter.IncreaseBy(1)
		require.NoError(t, err)
	}()

	// Counter first (it keeps the client reachable), then client and
	// connectivity once nothing references them anymore.
	gcEventually(t, func() bool {
		return rec.count("counter") == 1 && rec.count("client") == 1 &&
			rec.count("local_connectivity") == 1
	}, "all native handles must be released by cleanups")
}

func TestCleanupReleasesHandlerUserdata(t *testing.T) {
	rec := installLifecycleRecorder(t)

	// Default (no-op) connectivity: LocalConnectivity's server keeps an
	// Arc<WiredDatatype> (and with it the handler) until the key is
	// unsubscribed, so the handler chain is asserted without it.
	func() {
		client, err := NewClient("go-lifecycle-test", "gc-handler")
		require.NoError(t, err)
		counter, err := client.CreateCounter("gc-handler-counter", &DatatypeOptions{
			Handler: &Handler{OnStateChange: func(oldState, newState DatatypeState) {}},
		})
		require.NoError(t, err)
		_, err = counter.IncreaseBy(1)
		require.NoError(t, err)
	}()

	// Once both handles are cleaned up, the datatype drops, its event loop
	// stops, and Rust fires userdata_drop, deleting the handler's cgo.Handle.
	gcEventually(t, func() bool {
		return rec.count("counter") == 1 && rec.count("client") == 1 &&
			rec.count("handler_userdata") == 1
	}, "the handler userdata must be released after both handles are cleaned up")
}

func TestCloseStopsCleanupAndIsIdempotent(t *testing.T) {
	rec := installLifecycleRecorder(t)

	conn := NewLocalConnectivity()
	client, err := NewClient("go-lifecycle-test", "close-all", WithLocalConnectivity(conn))
	require.NoError(t, err)
	counter, err := client.CreateCounter("close-all-counter", nil)
	require.NoError(t, err)

	counter.Close()
	counter.Close() // double Close is a no-op
	client.Close()
	client.Close()
	conn.Close()
	conn.Close()

	// The wrappers are unreachable from here on; stopped cleanups must not fire
	// (a second free of the same handle would crash the process).
	gcSettle()
	require.Zero(t, rec.count("counter"), "explicit Close must stop the counter cleanup")
	require.Zero(t, rec.count("client"), "explicit Close must stop the client cleanup")
	require.Zero(t, rec.count("local_connectivity"), "explicit Close must stop the connectivity cleanup")
}

func TestCounterKeepsClientReachable(t *testing.T) {
	rec := installLifecycleRecorder(t)

	conn := NewLocalConnectivity()
	defer conn.Close()
	client, err := NewClient("go-lifecycle-test", "reach", WithLocalConnectivity(conn))
	require.NoError(t, err)
	counter, err := client.CreateCounter("reach-counter", nil)
	require.NoError(t, err)

	// Drop the only direct client reference; the counter must keep it alive.
	client = nil //nolint:ineffassign,wastedassign,staticcheck // deliberate: releases the test's reference
	gcSettle()
	require.Zero(t, rec.count("client"), "client must not be cleaned up while its counter is reachable")

	// The counter must still be fully functional, including syncing through the
	// (still alive) client.
	v, err := counter.IncreaseBy(3)
	require.NoError(t, err)
	require.EqualValues(t, 3, v)

	// Closing the counter severs the reference; the client may now be collected.
	counter.Close()
	gcEventually(t, func() bool { return rec.count("client") == 1 },
		"client cleanup must run once its last counter is closed")
}

func TestTransactionCounterRegistersNoCleanup(t *testing.T) {
	rec := installLifecycleRecorder(t)

	client, err := NewClient("go-lifecycle-test", "tx")
	require.NoError(t, err)
	defer client.Close()
	counter, err := client.CreateCounter("tx-counter", nil)
	require.NoError(t, err)
	defer counter.Close()

	require.NoError(t, counter.Transaction("tx-tag", func(tx *Counter) error {
		_, err := tx.IncreaseBy(2)
		return err
	}))

	// The tx-scoped wrapper is unreachable now. If a cleanup had been
	// registered for it, it would double-free the Rust-owned tx handle.
	gcSettle()
	require.Zero(t, rec.count("counter"), "tx-scoped counters must not register cleanups")
	require.EqualValues(t, 2, counter.Value())
}

func TestUseAfterCloseIsDefined(t *testing.T) {
	client, err := NewClient("go-lifecycle-test", "closed")
	require.NoError(t, err)
	counter, err := client.CreateCounter("closed-counter", nil)
	require.NoError(t, err)

	counter.Close()
	require.Zero(t, counter.Value())
	require.EqualValues(t, -1, int32(counter.State()))
	_, err = counter.IncreaseBy(1)
	require.Error(t, err)
	require.Error(t, counter.Sync())

	client.Close()
	_, err = client.CreateCounter("after-close", nil)
	require.Error(t, err)
}

func TestHandlerUserdataReleasedOnFailure(t *testing.T) {
	rec := installLifecycleRecorder(t)
	handler := &Handler{OnError: func(err *Error) {}}

	// Early FFI failure: the client handle is null, the counter build returns
	// before a handler could be registered.
	client, err := NewClient("go-lifecycle-test", "fail-early")
	require.NoError(t, err)
	client.Close()
	_, err = client.CreateCounter("fail-early-counter", &DatatypeOptions{Handler: handler})
	require.Error(t, err)
	require.Equal(t, 1, rec.count("handler_userdata"), "early build failure must release the handler userdata")

	// Late failure: re-registering an existing key fails inside build_counter,
	// and the builder (already holding the handler) is dropped in the FFI call.
	client2, err := NewClient("go-lifecycle-test", "fail-late")
	require.NoError(t, err)
	defer client2.Close()
	first, err := client2.CreateCounter("fail-late-counter", nil)
	require.NoError(t, err)
	defer first.Close()
	_, err = client2.CreateCounter("fail-late-counter", &DatatypeOptions{Handler: handler})
	require.Error(t, err)
	require.Equal(t, 2, rec.count("handler_userdata"), "failed build must release the handler userdata")

	// SetHandler on a closed counter: registration is impossible, Rust must
	// still fire userdata_drop exactly once.
	counter, err := client2.CreateCounter("fail-set-handler", nil)
	require.NoError(t, err)
	counter.Close()
	counter.SetHandler(0, handler)
	require.Equal(t, 3, rec.count("handler_userdata"), "SetHandler on a closed counter must release the handler userdata")
}
