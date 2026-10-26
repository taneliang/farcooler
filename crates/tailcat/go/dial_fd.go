//go:build !js

// The tunnel as a file descriptor, for the platforms that have them.
//
// Split from tailcat.go so the same package also builds for a browser
// (GOOS=js, main_js.go), which has no descriptors and no raw TCP: there a
// dialed connection goes to the page as an object, and this file's three
// pieces — the socketpair pump, the descriptor adoption and the relay's TCP
// probe — are replaced rather than carried. Everything a dial decides is in
// dialConn, which both builds share.
package main

import (
	"errors"
	"io"
	"net"
	"os"
	"strconv"
	"sync"
	"syscall"

	"github.com/tailscale/tailcat"
)

// dial opens a tunneled connection to a runner and hands back a descriptor.
//
// The failures are told apart because the spec asks for different sentences
// and an unrecognized client is ignored SILENTLY by the runner: the only thing
// a revoked device would otherwise get is a generic timeout, which is what
// this tree keeps committing fixes against.
//
//	EINVAL        the token or the key is not one this can use
//	EHOSTUNREACH  the relay the token names does not answer   → the rendezvous
//	ETIMEDOUT     the relay answers, the runner does not       → the runner
//	ENETDOWN      the local data plane would not come up       → us
//	ECONNREFUSED  the tunnel is up and the TCP connect through it failed
//
// The token is parsed here rather than left to the client, for two reasons: a
// corrupt token is the caller's mistake and must not be reported as the
// network, and the parse is what turns "our tokens always embed their region"
// from an assumption into a refusal. That invariant is load-bearing — a token
// that had to fetch a DERP map could fail on the first leg for a reason this
// would misattribute.
//
// The last row is worth reading exactly: OnTCP on the runner accepts before it
// dials sshd, so an sshd that is refusing gives a connect that SUCCEEDS and
// then an immediate EOF. ECONNREFUSED here means the TCP connect through the
// tunnel failed, not that SSH said no.
func dial(token, clientKey string, port uint16) (rc int) {
	defer recoverToErrno(&rc)
	conn, rc := dialConn(token, clientKey, port)
	if rc < 0 {
		return rc
	}
	fd, err := pumpToSocketpair(conn)
	if err != nil {
		conn.Conn.Close()
		conn.client.Close()
		return -int(syscall.EMFILE)
	}
	return fd
}

// pumpToSocketpair returns a file descriptor whose peer is wired to conn.
//
// Both directions are copied and either EOF tears the whole thing down, so a
// closed descriptor on the Rust side ends the goroutines and the tailcat conn
// rather than leaking them for the life of the process.
func pumpToSocketpair(conn net.Conn) (int, error) {
	fds, err := syscall.Socketpair(syscall.AF_UNIX, syscall.SOCK_STREAM, 0)
	if err != nil {
		return -1, err
	}
	ours, err := fdConn(fds[0])
	if err != nil {
		// The FAR end only. fdConn owns fds[0] whether or not it could adopt
		// it — see its doc — and a second close here would not be closing a
		// descriptor that is already gone: it would be closing whatever number
		// the runtime handed out in its place.
		syscall.Close(fds[1])
		return -1, err
	}
	var once sync.Once
	shut := func() { once.Do(func() { ours.Close(); conn.Close() }) }
	go func() { defer recoverInGoroutine(); io.Copy(ours, conn); shut() }()
	go func() { defer recoverInGoroutine(); io.Copy(conn, ours); shut() }()
	return fds[1], nil
}

// fdConn adopts a raw descriptor as a net.Conn, and it OWNS that descriptor
// EITHER WAY: on success the returned conn holds it; on failure os.NewFile has
// already taken it and the deferred Close has already run. A caller that closes
// it again on the failure path is not closing something that is merely gone —
// descriptor numbers are reused, so it closes whatever the runtime handed out
// in the meantime, which in this process is a live tunnel socket, the runner's
// key file, or a log. Nothing reports that; it surfaces later as a tunnel that
// died for no reason.
//
// A var, and only so that pumpToSocketpair's failure path is reachable from a
// test. Through a real socketpair it is not reachable at all: net.FileConn
// refuses only something that is not a socket, and syscall.Socketpair returns
// nothing else — which is exactly why the double close above sat here
// unnoticed and why no test could have gone red on it.
var fdConn = func(fd int) (net.Conn, error) {
	if fd < 0 {
		return nil, errors.New("not a file descriptor")
	}
	file := os.NewFile(uintptr(fd), "tailcat")
	if file == nil {
		// The one path where the descriptor is still ours: os.NewFile refused
		// it, so it adopted nothing, and returning here without closing would
		// leak it for the life of the process.
		syscall.Close(fd)
		return nil, errors.New("not a file descriptor")
	}
	defer file.Close() // FileConn dups; this closes our copy, not the peer's.
	return net.FileConn(file)
}

// relayReachable reports whether the DERP node a token names will accept a TCP
// connection.
//
// It exists because upstream's meow is fire-and-forget: magicsock queues the
// packet to its DERP writer and answers "sent" whether or not the relay is
// there, so a relay that is down and a runner that is ignoring you produce the
// same ten-second silence. Measured, not assumed — magicsock.sendAddr returns
// true as soon as the write lands on a channel. Without this probe the spec's
// "Can't reach the rendezvous service" is a sentence that can never be shown.
//
// It runs only after a handshake has already failed, so it costs nothing on
// the path anybody is waiting on. A relay that accepts TCP is not proof the
// DERP protocol is healthy — it is proof the network reaches it, which is what
// "check your connection" is about.
func relayReachable(ci tailcat.ConnInfo) bool {
	for _, region := range ci.Region {
		for _, node := range region.Nodes {
			port := node.DERPPort
			if port == 0 {
				port = 443
			}
			for _, host := range []string{node.HostName, node.IPv4, node.IPv6} {
				if host == "" {
					continue
				}
				conn, err := net.DialTimeout("tcp", net.JoinHostPort(host, strconv.Itoa(port)), relayProbeTimeout)
				if err == nil {
					conn.Close()
					return true
				}
			}
		}
	}
	return false
}
