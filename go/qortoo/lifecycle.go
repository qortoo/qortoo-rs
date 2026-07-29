package qortoo

/*
#include <qortoo.h>
*/
import "C"

import "sync/atomic"

// Native handles must be released with Close; the constructors additionally
// register a runtime.AddCleanup backstop that frees the handle once the
// wrapper becomes unreachable. Cleanups are not guaranteed to run (and never
// run at process exit), so they complement Close rather than replace it.
//
// The cleanup functions below receive only the native pointer: capturing the
// wrapper itself would keep it reachable forever and the cleanup would never
// run. The client cleanup shuts down the client's tokio runtime, which blocks
// the cleanup goroutine until in-flight handler callbacks return — one more
// reason handlers must not block.

// lifecycleHook, when set, observes native releases driven by GC cleanups and
// handler-userdata drops. Test-only; see lifecycle_test.go.
var lifecycleHook atomic.Pointer[func(event string)]

func notifyLifecycle(event string) {
	if f := lifecycleHook.Load(); f != nil {
		(*f)(event)
	}
}

func freeClientPtr(p *C.QortooClient) {
	C.qortoo_client_free(p)
	notifyLifecycle("client")
}

func freeCounterPtr(p *C.QortooCounter) {
	C.qortoo_counter_free(p)
	notifyLifecycle("counter")
}

func freeLocalConnectivityPtr(p *C.QortooLocalConnectivity) {
	C.qortoo_local_connectivity_free(p)
	notifyLifecycle("local_connectivity")
}
