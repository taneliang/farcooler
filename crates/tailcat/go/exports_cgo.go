// The C entry points the iOS and macOS builds link against.
//
// Split out of `tailcat.go` so that the rest of this package builds with
// `CGO_ENABLED=0`. A file that imports "C" is excluded from the build
// whenever cgo is off — that implicit constraint is the whole mechanism — so
// this file, and only this file, is what makes the package a c-archive.
//
// Why that matters: a Go c-archive linked into a **musl** binary segfaults
// inside Go's runtime startup, before it can print a word. Measured, not
// reasoned: a five-line Go program reproduces it, static and dynamic alike,
// while both glibc forms work, and raising the thread stack through
// `PT_GNU_STACK` (8, 16, 64 and 256 MB, each verified with `readelf -l`)
// changes nothing. Linux runners therefore do not link this archive at all —
// they run `main_helper.go`'s standalone, cgo-free program instead, which uses
// no C library and so cannot hit it. See `.claude/agent/done/
// keep-musl-and-get-the-tunnel.md` and `docs/releasing.md`.
//
// iOS and macOS keep the archive. It works there and it ships today.
package main

/*
#include <stdint.h>
*/
import "C"

import (
	"syscall"
	"unsafe"
)

//export fc_tailcat_dial
func fc_tailcat_dial(token, clientKey *C.char, port C.uint16_t) C.int {
	return C.int(dial(C.GoString(token), C.GoString(clientKey), uint16(port)))
}

//export fc_tailcat_serve
func fc_tailcat_serve(keyPath *C.char, sshPort C.uint16_t, allow *C.char) C.int {
	return C.int(serve(C.GoString(keyPath), uint16(sshPort), C.GoString(allow)))
}

//export fc_tailcat_conn_blob
func fc_tailcat_conn_blob(buf *C.char, length C.size_t) C.int {
	if buf == nil || length == 0 {
		return C.int(-int(syscall.EINVAL))
	}
	return C.int(connBlob(unsafe.Slice((*byte)(unsafe.Pointer(buf)), int(length))))
}

//export fc_tailcat_allow_add
func fc_tailcat_allow_add(nodeKey *C.char) C.int {
	return C.int(allowAdd(C.GoString(nodeKey)))
}

//export fc_tailcat_set_derp_map_url
func fc_tailcat_set_derp_map_url(url *C.char) C.int {
	setDerpMapURL(C.GoString(url))
	return 0
}

// A c-archive never runs main. It is here because a `package main` must have
// one; `main_helper.go` carries the one that does something, for the build
// where this file is absent.
func main() {}
