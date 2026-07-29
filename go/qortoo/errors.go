package qortoo

/*
#include <qortoo.h>
*/
import "C"

import "fmt"

// Error codes mirroring the #[repr(i32)] discriminants of the Rust error enums.
const (
	// ClientError (100–)
	ErrCodeInvalidCollectionName             int32 = 100
	ErrCodeFailedToSubscribeOrCreateDatatype int32 = 101

	// DatatypeError (200–)
	ErrCodeTransactionFailed            int32 = 201
	ErrCodeInternal                     int32 = 202
	ErrCodeDisallowed                   int32 = 205
	ErrCodeNotWritable                  int32 = 206
	ErrCodeReadonlyViolation            int32 = 207
	ErrCodeSyncFailed                   int32 = 210
	ErrCodePushBufferExceededMaxMemSize int32 = 211
	ErrCodeServerRejected               int32 = 213

	// FFI boundary (see qortoo-ffi)
	ErrCodeInvalidArgument int32 = 998
	ErrCodeInternalFFI     int32 = 999
)

// Error is an error surfaced from the Qortoo SDK across the FFI boundary.
//
// Two errors with the same Code represent the same error variant; Msg is
// informational only (matching the variant-only equality of the Rust enums).
type Error struct {
	Code int32
	Msg  string
}

func (e *Error) Error() string {
	return fmt.Sprintf("qortoo: %s (code %d)", e.Msg, e.Code)
}

// Is reports code-based equality so errors.Is(err, &Error{Code: ...}) works.
func (e *Error) Is(target error) bool {
	t, ok := target.(*Error)
	return ok && t.Code == e.Code
}

// takeError converts a QortooError out-parameter into a Go error (nil on success)
// and releases the message string.
func takeError(cerr *C.QortooError) error {
	if cerr.code == 0 {
		return nil
	}
	return &Error{Code: int32(cerr.code), Msg: goString(cerr.msg)}
}
