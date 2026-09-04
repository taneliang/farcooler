// The tunnel as its own process, for Linux runners.
//
// A Go c-archive linked into a musl binary segfaults inside Go's runtime
// startup before it can print a word — measured, and not this code; see
// `exports_cgo.go`. A standalone Go program built `CGO_ENABLED=0` links no C
// library at all, so the problem does not arise rather than being worked
// around, and the binary still runs on every distribution, which is the
// property musl was chosen for in the first place.
//
// The data path is unchanged and the daemon is not in it either way: bytes go
// from this process's netstack straight to sshd on loopback, exactly as they
// did from inside the daemon. What the pipe below carries is control — start
// serving, admit one more device, what is my token — three messages a runner
// sends a handful of times in its life.
//
// It reads its commands from STDIN and answers on STDOUT, one line each, and
// that is deliberately not a socket. A file descriptor the parent already
// holds is not addressable: nothing else on the host can reach it, there is no
// port to guess and no path to race, and the helper dies on its own when the
// daemon does, because that is what EOF on stdin means. A loopback port would
// have given every other process on the runner a way to ask this program to
// admit a node key.
//
// This is the SERVING half only. Dialing a tunnel from Linux is a later task —
// `farcooler-cli` does not link tailcat yet (`scripts/build-linux.sh`) — and
// this does not solve it: `dial` hands its caller a file descriptor, and a
// descriptor cannot cross this pipe as a word. Whoever wires Linux dialing has
// to answer the fd-passing question (SCM_RIGHTS over a Unix socketpair,
// presumably) rather than extending the line protocol below.
//
// Nothing here authenticates anybody, exactly as nothing in `tailcat.go` does.
// SSH still authenticates, `authorized_keys` still forces the command and the
// scope, and the fence still owns that file.
package main

import (
	"bufio"
	"fmt"
	"io"
	"strconv"
	"strings"
	"syscall"
)

// helperBlobBuf is the buffer `blob` reads a token into. A production ConnBlob
// measured at 184 bytes; this matches the Rust side's own 1024 rather than
// sizing to that one measurement, so the two agree about when a token is too
// long to carry.
const helperBlobBuf = 1024

// runHelper reads commands until EOF and answers each one.
//
// One command at a time, deliberately: `serve` can take tens of seconds and
// holds the package mutex for all of it, so pipelining would buy nothing but a
// second way for replies to arrive out of order. A runner sends these a
// handful of times in its life.
//
// Every reply is one line: "ok", "ok <payload>", or "err <errno>", where the
// errno is the positive form of what the function underneath returned. The
// Rust side turns it back into the same `io::Error::from_raw_os_error` the
// linked backend produces from the same call, so both backends fail
// identically.
func runHelper(keyPath string, in io.Reader, out io.Writer) error {
	reader := bufio.NewReader(in)
	writer := bufio.NewWriter(out)
	for {
		line, err := reader.ReadString('\n')
		if line == "" && err != nil {
			// EOF is how this program is asked to stop: the daemon exited, or
			// closed the pipe on purpose. Not an error.
			if err == io.EOF {
				return nil
			}
			return err
		}
		reply := handle(keyPath, strings.TrimRight(line, "\r\n"))
		if _, werr := fmt.Fprintln(writer, reply); werr != nil {
			return werr
		}
		// Flushed per reply, not per batch. The daemon is blocked reading this
		// line and a buffered reply would deadlock both processes.
		if ferr := writer.Flush(); ferr != nil {
			return ferr
		}
		if err != nil {
			if err == io.EOF {
				return nil
			}
			return err
		}
	}
}

// handle runs one command and returns the line to answer with.
func handle(keyPath, line string) string {
	fields := strings.Fields(line)
	if len(fields) == 0 {
		return errno(syscall.EINVAL)
	}
	switch fields[0] {
	// serve <ssh port> <node key>...
	//
	// The allowlist arrives as arguments rather than being read from a file
	// here, for the same reason the linked backend passes it: the runner's
	// `authorized_keys` is the single source of who is admitted, and a second
	// reader of it would be a second place that truth can be wrong.
	case "serve":
		if len(fields) < 3 {
			return errno(syscall.EINVAL)
		}
		port, err := strconv.ParseUint(fields[1], 10, 16)
		if err != nil {
			return errno(syscall.EINVAL)
		}
		// Newline-separated, which is what parseAllowed splits on, and an
		// empty list can never be built here: `len(fields) < 3` above already
		// refused it. serve() refuses it again — tailcat reads an empty
		// AllowedClients as "admit everyone", so this is a guard worth having
		// twice.
		return rc(serve(keyPath, uint16(port), strings.Join(fields[2:], "\n")))
	// allow <node key>
	case "allow":
		if len(fields) != 2 {
			return errno(syscall.EINVAL)
		}
		return rc(allowAdd(fields[1]))
	// blob
	case "blob":
		if len(fields) != 1 {
			return errno(syscall.EINVAL)
		}
		buf := make([]byte, helperBlobBuf)
		n := connBlob(buf)
		if n < 0 {
			return rc(n)
		}
		return "ok " + string(buf[:n])
	// derpmap <url>
	//
	// Set before serving, never after: the URL is read when the server is
	// built. An empty URL is the library default and cannot be spelled as a
	// field here, which is fine — a caller that wants the default never sends
	// this command at all.
	case "derpmap":
		if len(fields) != 2 {
			return errno(syscall.EINVAL)
		}
		setDerpMapURL(fields[1])
		return "ok"
	default:
		return errno(syscall.EINVAL)
	}
}

// rc turns one of this package's negative-errno returns into a reply line.
func rc(n int) string {
	if n < 0 {
		return fmt.Sprintf("err %d", -n)
	}
	return "ok"
}

func errno(e syscall.Errno) string {
	return fmt.Sprintf("err %d", int(e))
}
