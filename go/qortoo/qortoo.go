package qortoo

/*
#include <qortoo.h>
*/
import "C"

import (
	"errors"
	"runtime"
)

// DataType identifies the kind of CRDT datatype (mirrors qortoo::DataType).
type DataType int32

const (
	TypeCounter  DataType = 0
	TypeVariable DataType = 1
	TypeMap      DataType = 2
)

// DatatypeState mirrors qortoo::DatatypeState (lifecycle and write access).
type DatatypeState int32

const (
	StateCreating              DatatypeState = 0
	StateSubscribing           DatatypeState = 1
	StateSubscribingOrCreating DatatypeState = 2
	StateSubscribed            DatatypeState = 3
	StateUnsubscribing         DatatypeState = 4
	StateDeleting              DatatypeState = 5
	StateDisabled              DatatypeState = 6
)

func (s DatatypeState) String() string {
	switch s {
	case StateCreating:
		return "Creating"
	case StateSubscribing:
		return "Subscribing"
	case StateSubscribingOrCreating:
		return "SubscribingOrCreating"
	case StateSubscribed:
		return "Subscribed"
	case StateUnsubscribing:
		return "Unsubscribing"
	case StateDeleting:
		return "Deleting"
	case StateDisabled:
		return "Disabled"
	default:
		return "Unknown"
	}
}

// LocalConnectivity is an in-memory synchronization backend for testing and
// development, mirroring the Rust core type of the same name. Share one
// instance between clients to synchronize them.
type LocalConnectivity struct {
	ptr     *C.QortooLocalConnectivity
	cleanup runtime.Cleanup
}

// NewLocalConnectivity creates an in-memory backend in realtime mode.
func NewLocalConnectivity() *LocalConnectivity {
	l := &LocalConnectivity{ptr: C.qortoo_local_connectivity_new()}
	l.cleanup = runtime.AddCleanup(l, freeLocalConnectivityPtr, l.ptr)
	return l
}

// SetRealtime toggles between realtime (auto-sync) and manual (explicit Sync) mode.
func (l *LocalConnectivity) SetRealtime(realtime bool) {
	defer runtime.KeepAlive(l)
	C.qortoo_local_connectivity_set_realtime(l.ptr, C.bool(realtime))
}

// Close releases the native handle. Clients built with this connectivity keep
// their own reference and stay functional. Close must not be called
// concurrently with other methods on the same object.
func (l *LocalConnectivity) Close() {
	if l.ptr != nil {
		l.cleanup.Stop()
		C.qortoo_local_connectivity_free(l.ptr)
		l.ptr = nil
	}
}

// Client is the entry point of the SDK, scoped by a collection name and alias.
type Client struct {
	ptr     *C.QortooClient
	cleanup runtime.Cleanup
}

type clientConfig struct {
	local *LocalConnectivity
}

// ClientOption configures NewClient.
type ClientOption func(*clientConfig)

// WithLocalConnectivity attaches a shared in-memory backend to the client.
func WithLocalConnectivity(conn *LocalConnectivity) ClientOption {
	return func(cfg *clientConfig) { cfg.local = conn }
}

// NewClient builds a client. Without a connectivity option it uses the
// SDK-default no-op backend.
func NewClient(collection, alias string, opts ...ClientOption) (*Client, error) {
	var cfg clientConfig
	for _, opt := range opts {
		opt(&cfg)
	}
	// The connectivity wrapper provides a native pointer to the cgo call below;
	// keep it alive so its GC cleanup cannot free that pointer mid-call.
	defer runtime.KeepAlive(cfg.local)

	cCollection := cString(collection)
	defer freeCString(cCollection)
	cAlias := cString(alias)
	defer freeCString(cAlias)

	var cerr C.QortooError
	var ptr *C.QortooClient
	if cfg.local != nil {
		if cfg.local.ptr == nil {
			return nil, errors.New("qortoo: LocalConnectivity is closed")
		}
		ptr = C.qortoo_client_new(cCollection, cAlias, cfg.local.ptr, &cerr)
	} else {
		ptr = C.qortoo_client_new(cCollection, cAlias, nil, &cerr)
	}
	if err := takeError(&cerr); err != nil {
		return nil, err
	}
	cl := &Client{ptr: ptr}
	cl.cleanup = runtime.AddCleanup(cl, freeClientPtr, ptr)
	return cl, nil
}

// Collection returns the collection name of this client.
func (c *Client) Collection() string {
	defer runtime.KeepAlive(c)
	return goString(C.qortoo_client_get_collection(c.ptr))
}

// Alias returns the alias of this client.
func (c *Client) Alias() string {
	defer runtime.KeepAlive(c)
	return goString(C.qortoo_client_get_alias(c.ptr))
}

// UnsubscribeDatatype marks the datatype identified by key as unsubscribing.
// With manual connectivity, a following Counter.Sync drives it to StateDisabled.
func (c *Client) UnsubscribeDatatype(key string) error {
	defer runtime.KeepAlive(c)
	cKey := cString(key)
	defer freeCString(cKey)
	var cerr C.QortooError
	C.qortoo_client_unsubscribe_datatype(c.ptr, cKey, &cerr)
	return takeError(&cerr)
}

// Close releases the client and shuts down its internal runtime. Counters
// created from it remain safe to use until their own Close, but can no longer
// synchronize. Close must not be called concurrently with other methods on the
// same object.
func (c *Client) Close() {
	if c.ptr != nil {
		c.cleanup.Stop()
		C.qortoo_client_free(c.ptr)
		c.ptr = nil
	}
}
