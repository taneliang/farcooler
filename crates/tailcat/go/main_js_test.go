//go:build js

// Run in a JS engine: `GOOS=js GOARCH=wasm go test -exec "$(go env GOROOT)/lib/wasm/go_js_wasm_exec" .`
// with node on PATH. CI's web job does.
package main

import (
	"bytes"
	"io"
	"net"
	"sync"
	"syscall"
	"syscall/js"
	"testing"
	"time"

	"github.com/tailscale/tailcat"
)

// Every write the page makes reaches the runner in the order it was made.
//
// spike-761 found SSH's bytes reordered when each write ran on a goroutine of
// its own: the tunnel's Write blocks for as long as its stack takes, so two
// waiting writers finish in whatever order the stack frees them, and one
// keystroke overtaking another is a corrupt packet that ends the connection.
// slowConn makes that visible: its Write takes longer for some sizes than
// others, so writers running side by side finish out of order. This drives the
// real write function a page calls.
func TestWritesReachTheTunnelInTheOrderThePageMadeThem(t *testing.T) {
	slow := &slowConn{}
	conn := jsConn(&clientConn{Conn: slow})

	var want []byte
	for i := 0; i < 60; i++ {
		chunk := bytes.Repeat([]byte{byte(i)}, 1+i%3)
		want = append(want, chunk...)
		u8 := js.Global().Get("Uint8Array").New(len(chunk))
		js.CopyBytesToJS(u8, chunk)
		conn.Call("write", u8)
	}

	deadline := time.Now().Add(10 * time.Second)
	for len(slow.written()) < len(want) && time.Now().Before(deadline) {
		time.Sleep(5 * time.Millisecond)
	}
	if got := slow.written(); !bytes.Equal(got, want) {
		t.Fatalf("the tunnel carried the page's writes out of order:\n got %v\nwant %v", got, want)
	}
}

// slowConn is a connection whose Write takes 1, 2 or 3 ms by the size of what
// it is given, and keeps what it was given in the order its writes finished.
type slowConn struct {
	net.Conn
	mu  sync.Mutex
	out []byte
}

func (c *slowConn) Write(p []byte) (int, error) {
	time.Sleep(time.Duration(4-len(p)) * time.Millisecond)
	c.mu.Lock()
	defer c.mu.Unlock()
	c.out = append(c.out, p...)
	return len(p), nil
}

func (c *slowConn) written() []byte {
	c.mu.Lock()
	defer c.mu.Unlock()
	return append([]byte(nil), c.out...)
}

// A write copies its bytes before it returns, so a caller that reuses its
// buffer (russh hands over a view into wasm memory) cannot change what is sent.
func TestAWriteCopiesTheBytesItWasGiven(t *testing.T) {
	near, far := net.Pipe()
	defer far.Close()
	conn := jsConn(&clientConn{Conn: near})

	u8 := js.Global().Get("Uint8Array").New(3)
	js.CopyBytesToJS(u8, []byte("abc"))
	conn.Call("write", u8)
	js.CopyBytesToJS(u8, []byte("xyz"))

	got := make([]byte, 3)
	far.SetReadDeadline(time.Now().Add(10 * time.Second))
	if _, err := io.ReadFull(far, got); err != nil {
		t.Fatalf("read: %v", err)
	}
	if string(got) != "abc" {
		t.Fatalf("sent %q, want the bytes as they were at the call, %q", got, "abc")
	}
}

// The page shows a sentence per word, so each failure dialConn tells apart
// must arrive as its own word, and the two a phone already has keep a phone's
// spelling (crates/tailcat/src/lib.rs, TunnelError::code).
func TestEachDialFailureHasTheWordAPhoneUses(t *testing.T) {
	cases := map[syscall.Errno]string{
		syscall.EHOSTUNREACH: "derp",
		syscall.ETIMEDOUT:    "no_answer",
		syscall.EINVAL:       "io",
		syscall.ENETDOWN:     "io",
		syscall.ECONNREFUSED: "io",
	}
	for errno, want := range cases {
		if got := dialWord(errno); got != want {
			t.Errorf("dialWord(%v) = %q, want %q", errno, got, want)
		}
	}
}

// A browser cannot probe the relay over TCP, and must not blame it for a
// handshake that went nowhere: a revoked device would be told to check its
// connection.
func TestABrowserNeverBlamesTheRelayItCannotProbe(t *testing.T) {
	if !relayReachable(tailcat.ConnInfo{}) {
		t.Fatal("relayReachable said the relay is down, which a browser cannot know")
	}
}
