package qortoo

// This file holds the //export trampolines invoked from Rust. cgo forbids
// definitions in the preamble of files containing //export, so the fn-pointer
// getters live in cgo.go.

/*
#include <qortoo.h>
*/
import "C"

import (
	"fmt"
	"runtime/cgo"
)

// handlerFromUserdata resolves the cgo.Handle passed through the uintptr_t
// userdata. Returns nil when userdata does not carry a *Handler.
func handlerFromUserdata(userdata C.uintptr_t) *Handler {
	if userdata == 0 {
		return nil
	}
	h, ok := cgo.Handle(userdata).Value().(*Handler)
	if !ok {
		return nil
	}
	return h
}

//export goQortooOnStateChange
func goQortooOnStateChange(userdata C.uintptr_t, oldState, newState C.int32_t) {
	// A Go panic must not unwind into the Rust worker thread.
	defer func() { _ = recover() }()
	if h := handlerFromUserdata(userdata); h != nil && h.OnStateChange != nil {
		h.OnStateChange(DatatypeState(oldState), DatatypeState(newState))
	}
}

//export goQortooOnError
func goQortooOnError(userdata C.uintptr_t, code C.int32_t, msg *C.char) {
	defer func() { _ = recover() }()
	if h := handlerFromUserdata(userdata); h != nil && h.OnError != nil {
		// msg is owned by Rust and only valid during this call: copy, don't free.
		h.OnError(&Error{Code: int32(code), Msg: C.GoString(msg)})
	}
}

//export goQortooUserdataDrop
func goQortooUserdataDrop(userdata C.uintptr_t) {
	if userdata != 0 {
		cgo.Handle(userdata).Delete()
		notifyLifecycle("handler_userdata")
	}
}

//export goQortooTxCallback
func goQortooTxCallback(txCounter *C.QortooCounter, userdata C.uintptr_t) (ret C.int32_t) {
	txc, ok := cgo.Handle(userdata).Value().(*txContext)
	if !ok {
		return 1
	}
	defer func() {
		if r := recover(); r != nil {
			txc.err = fmt.Errorf("qortoo: transaction panicked: %v", r)
			ret = 1
		}
	}()
	tx := &Counter{ptr: txCounter, borrowed: true}
	if err := txc.fn(tx); err != nil {
		txc.err = err
		return 1
	}
	return 0
}
