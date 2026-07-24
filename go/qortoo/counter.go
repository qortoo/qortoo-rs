package qortoo

/*
#include <qortoo.h>

// Fn-pointer getters defined in cgo.go's preamble (external linkage).
extern QortooOnStateChangeCallback qortooGoStateChangeCB(void);
extern QortooOnErrorCallback qortooGoErrorCB(void);
extern QortooUserdataDropCallback qortooGoUserdataDropCB(void);
extern QortooTxCallback qortooGoTxCB(void);
*/
import "C"

import (
	"runtime"
	"runtime/cgo"
)

// Handler receives datatype notifications. Both fields are optional.
// Callbacks run on Qortoo-owned worker threads (see package docs).
type Handler struct {
	// OnStateChange is called as (oldState, newState) after a lifecycle transition.
	OnStateChange func(oldState, newState DatatypeState)
	// OnError is called with errors surfaced from the sync path.
	OnError func(err *Error)
}

// DatatypeOptions configures counter construction. The zero value (or nil
// pointer) selects the SDK defaults.
type DatatypeOptions struct {
	// Readonly rejects all local writes regardless of state.
	Readonly bool
	// MaxPushBufferSize in bytes; 0 keeps the SDK default (clamped by the SDK).
	MaxPushBufferSize uint64
	// Handler registered at build time (needed to observe the first auto-sync
	// with realtime connectivity). HandlerPriority orders multiple handlers.
	Handler         *Handler
	HandlerPriority uint
}

// Counter is a conflict-free counter datatype.
type Counter struct {
	ptr *C.QortooCounter
	// client keeps the owning Client reachable so its GC cleanup cannot shut
	// the native client down while this counter is still in use. Severed by
	// Close; nil for transaction-scoped handles.
	client *Client
	// borrowed marks transaction-scoped handles owned by Rust (must not be freed).
	borrowed bool
	cleanup  runtime.Cleanup
}

func (opts *DatatypeOptions) toC() *C.QortooDatatypeOptions {
	if opts == nil {
		return nil
	}
	c := &C.QortooDatatypeOptions{
		readonly:             C.bool(opts.Readonly),
		max_push_buffer_size: C.uint64_t(opts.MaxPushBufferSize),
		handler_priority:     C.uintptr_t(opts.HandlerPriority),
	}
	if opts.Handler != nil {
		h := cgo.NewHandle(opts.Handler)
		c.on_state_change = C.qortooGoStateChangeCB()
		c.on_error = C.qortooGoErrorCB()
		// The handle is released by goQortooUserdataDrop when Rust drops the handler.
		c.handler_userdata = C.uintptr_t(h)
		c.handler_userdata_drop = C.qortooGoUserdataDropCB()
	}
	return c
}

func (c *Client) buildCounter(
	key string,
	opts *DatatypeOptions,
	build func(*C.QortooClient, *C.char, *C.QortooDatatypeOptions, *C.QortooError) *C.QortooCounter,
) (*Counter, error) {
	defer runtime.KeepAlive(c)
	cKey := cString(key)
	defer freeCString(cKey)
	var cerr C.QortooError
	ptr := build(c.ptr, cKey, opts.toC(), &cerr)
	if err := takeError(&cerr); err != nil {
		return nil, err
	}
	ctr := &Counter{ptr: ptr, client: c}
	ctr.cleanup = runtime.AddCleanup(ctr, freeCounterPtr, ptr)
	return ctr, nil
}

// CreateCounter builds a counter in StateCreating (writable).
func (c *Client) CreateCounter(key string, opts *DatatypeOptions) (*Counter, error) {
	return c.buildCounter(key, opts, func(cl *C.QortooClient, k *C.char, o *C.QortooDatatypeOptions, e *C.QortooError) *C.QortooCounter {
		return C.qortoo_counter_create(cl, k, o, e)
	})
}

// SubscribeCounter builds a counter in StateSubscribing (read-only until synced).
func (c *Client) SubscribeCounter(key string, opts *DatatypeOptions) (*Counter, error) {
	return c.buildCounter(key, opts, func(cl *C.QortooClient, k *C.char, o *C.QortooDatatypeOptions, e *C.QortooError) *C.QortooCounter {
		return C.qortoo_counter_subscribe(cl, k, o, e)
	})
}

// SubscribeOrCreateCounter builds a counter in StateSubscribingOrCreating (writable).
func (c *Client) SubscribeOrCreateCounter(key string, opts *DatatypeOptions) (*Counter, error) {
	return c.buildCounter(key, opts, func(cl *C.QortooClient, k *C.char, o *C.QortooDatatypeOptions, e *C.QortooError) *C.QortooCounter {
		return C.qortoo_counter_subscribe_or_create(cl, k, o, e)
	})
}

// IncreaseBy adds delta (which may be negative) and returns the new value.
func (c *Counter) IncreaseBy(delta int64) (int64, error) {
	defer runtime.KeepAlive(c)
	var cerr C.QortooError
	v := C.qortoo_counter_increase_by(c.ptr, C.int64_t(delta), &cerr)
	if err := takeError(&cerr); err != nil {
		return 0, err
	}
	return int64(v), nil
}

// Increase adds 1 and returns the new value.
func (c *Counter) Increase() (int64, error) {
	return c.IncreaseBy(1)
}

// Value returns the current counter value.
func (c *Counter) Value() int64 {
	defer runtime.KeepAlive(c)
	return int64(C.qortoo_counter_get_value(c.ptr))
}

// Sync performs a blocking push/pull with the connectivity backend.
func (c *Counter) Sync() error {
	defer runtime.KeepAlive(c)
	var cerr C.QortooError
	C.qortoo_counter_sync(c.ptr, &cerr)
	return takeError(&cerr)
}

// Unsubscribe marks this datatype as unsubscribing (see Client.UnsubscribeDatatype).
func (c *Counter) Unsubscribe() error {
	defer runtime.KeepAlive(c)
	var cerr C.QortooError
	C.qortoo_counter_unsubscribe(c.ptr, &cerr)
	return takeError(&cerr)
}

// Key returns the datatype key.
func (c *Counter) Key() string {
	defer runtime.KeepAlive(c)
	return goString(C.qortoo_counter_get_key(c.ptr))
}

// Type returns the datatype kind (always TypeCounter for a Counter).
func (c *Counter) Type() DataType {
	defer runtime.KeepAlive(c)
	return DataType(C.qortoo_counter_get_type(c.ptr))
}

// State returns the current lifecycle state.
func (c *Counter) State() DatatypeState {
	defer runtime.KeepAlive(c)
	return DatatypeState(C.qortoo_counter_get_state(c.ptr))
}

// ServerVersion returns the server-side version (0 before the first sync).
func (c *Counter) ServerVersion() uint64 {
	defer runtime.KeepAlive(c)
	return uint64(C.qortoo_counter_get_server_version(c.ptr))
}

// ClientVersion returns the number of local operations.
func (c *Counter) ClientVersion() uint64 {
	defer runtime.KeepAlive(c)
	return uint64(C.qortoo_counter_get_client_version(c.ptr))
}

// SyncedClientVersion returns the last client version acknowledged by the server.
func (c *Counter) SyncedClientVersion() uint64 {
	defer runtime.KeepAlive(c)
	return uint64(C.qortoo_counter_get_synced_client_version(c.ptr))
}

// SetHandler registers (or replaces) a handler at the given priority
// (lower priority runs first).
func (c *Counter) SetHandler(priority uint, h *Handler) {
	defer runtime.KeepAlive(c)
	// If registration fails (e.g., the counter is closed), Rust still fires
	// userdata_drop exactly once, releasing this handle.
	hh := cgo.NewHandle(h)
	C.qortoo_counter_set_handler(
		c.ptr,
		C.uintptr_t(priority),
		C.qortooGoStateChangeCB(),
		C.qortooGoErrorCB(),
		C.uintptr_t(hh),
		C.qortooGoUserdataDropCB(),
	)
}

// UnsetHandler removes the handler at the given priority. Returns true if one
// was removed.
func (c *Counter) UnsetHandler(priority uint) bool {
	defer runtime.KeepAlive(c)
	return bool(C.qortoo_counter_unset_handler(c.ptr, C.uintptr_t(priority)))
}

// txContext carries the transaction body and its error across the FFI boundary.
type txContext struct {
	fn  func(tx *Counter) error
	err error
}

// Transaction executes fn atomically: if fn returns an error (or panics), every
// operation performed through tx is rolled back. fn runs inline on the calling
// goroutine; tx is only valid during the call.
func (c *Counter) Transaction(tag string, fn func(tx *Counter) error) error {
	defer runtime.KeepAlive(c)
	cTag := cString(tag)
	defer freeCString(cTag)

	txc := &txContext{fn: fn}
	h := cgo.NewHandle(txc)
	defer h.Delete()

	var cerr C.QortooError
	C.qortoo_counter_transaction(c.ptr, cTag, C.qortooGoTxCB(), C.uintptr_t(h), &cerr)
	if err := takeError(&cerr); err != nil {
		if txc.err != nil {
			return txc.err
		}
		return err
	}
	return nil
}

// Close releases this handle. The underlying datatype stays registered in its
// client. No-op for transaction-scoped handles. Close must not be called
// concurrently with other methods on the same object.
func (c *Counter) Close() {
	if c.ptr != nil && !c.borrowed {
		c.cleanup.Stop()
		C.qortoo_counter_free(c.ptr)
	}
	c.ptr = nil
	c.client = nil
}
