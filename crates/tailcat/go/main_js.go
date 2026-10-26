//go:build js

// The tunnel's client half for a browser (ov-764): this package, built with
// GOOS=js GOARCH=wasm, dials a runner the way a phone does and hands the page
// the connection as an object.
//
// The same package rather than a copy in a package of its own, so a browser
// and a runner are always the same Tailcat by construction: one go.mod, one
// dialConn, one node-key encoding. What a browser cannot do is replaced in
// this file and nothing else:
//
//   - No file descriptors, so no socketpair. A dialed connection is an object
//     with read, write and close, which the Rust side (crates/web) wraps as a
//     byte stream for russh.
//   - No raw TCP, so the relay probe that tells "the rendezvous is down" from
//     "the runner did not answer" cannot run. See relayReachable below.
//   - No main loop of its own: main registers the functions and parks, and
//     every call that waits runs on a goroutine of its own, because a
//     js.FuncOf callback that blocks deadlocks the page.
//
// DERP is reached over WebSocket: under GOOS=js upstream's derphttp uses
// nothing else. The page's CSP names the hosts it may reach.
//
// As on every other platform, nothing here authenticates anybody. SSH does,
// against a host key the page pinned from the enrollment reply.
package main

import (
	"errors"
	"io"
	"sync"
	"syscall"
	"syscall/js"

	"github.com/tailscale/tailcat"
)

// main publishes `globalThis.fcTailcat` and tells the page it is ready.
//
// The page starts the module and waits for `fcTailcatReady`, rather than
// polling for the global: a callback is one moment, a poll is a guess.
func main() {
	api := js.Global().Get("Object").New()
	api.Set("mintNodeKey", js.FuncOf(jsMintNodeKey))
	api.Set("dial", js.FuncOf(jsDial))
	js.Global().Set("fcTailcat", api)
	if ready := js.Global().Get("fcTailcatReady"); ready.Type() == js.TypeFunction {
		ready.Invoke()
	}
	select {}
}

// relayReachable always answers yes in a browser, which cannot open a raw TCP
// connection to ask.
//
// Yes rather than no on purpose. "No" would turn every failed handshake into
// "can't reach the rendezvous", including a revoked device's, which is the
// misreport dialConn's split exists to prevent. "Yes" leaves the two answers a
// browser can honestly give: the runner did not answer, or something else
// failed.
func relayReachable(tailcat.ConnInfo) bool {
	return true
}

// jsMintNodeKey answers {priv, pub}: a fresh node key pair in this product's
// 43-character encoding, made by the same mintIdentity a phone's key comes
// from. The page keeps the private half; only the public half is ever shown.
func jsMintNodeKey(js.Value, []js.Value) any {
	id := mintIdentity()
	return js.ValueOf(map[string]any{
		"priv": encodeNodeKey(id.priv.Raw32()),
		"pub":  encodeNodeKey(id.priv.Public().Raw32()),
	})
}

// jsDial is `dial(token, key) → Promise<conn>`.
//
// A refusal is an Error whose message is one stable word, the same words the
// Rust side's TunnelError::code gives a phone, so the page owns the sentence:
//
//	derp       the relay the token names did not answer (never, in a browser;
//	           see relayReachable)
//	no_answer  the relay answered and the runner did not, which is also what
//	           a device the runner no longer admits sees
//	io         anything else: a bad token or key, the data plane, the connect
func jsDial(_ js.Value, args []js.Value) any {
	if len(args) < 2 || args[0].Type() != js.TypeString || args[1].Type() != js.TypeString {
		return rejected("io")
	}
	token, key := args[0].String(), args[1].String()
	return promise(func() (any, error) {
		conn, rc := dialConn(token, key, tunnelPort)
		if rc < 0 {
			return nil, errors.New(dialWord(syscall.Errno(-rc)))
		}
		return jsConn(conn), nil
	})
}

// dialWord maps dialConn's errno to the word the page shows a sentence for.
func dialWord(e syscall.Errno) string {
	switch e {
	case syscall.EHOSTUNREACH:
		return "derp"
	case syscall.ETIMEDOUT:
		return "no_answer"
	default:
		return "io"
	}
}

// jsConn hands a dialed connection to the page as {read, write, close}.
//
//   - read() → Promise<Uint8Array | null>. Null is the end of the stream. The
//     caller awaits one read before asking for the next; crates/web does.
//   - write(bytes) copies the bytes before it returns (the caller may hand a
//     view into its own wasm memory) and queues them for ONE writer goroutine.
//     A goroutine per write does not keep SSH's bytes in order (spike-761),
//     and a write that waited would deadlock the page, so it never waits.
//   - close() ends both directions and the tunnel's client, once.
func jsConn(conn *clientConn) js.Value {
	out := newOutbox()
	go func() {
		defer recoverInGoroutine()
		for {
			batch, open := out.take()
			for _, b := range batch {
				if _, err := conn.Write(b); err != nil {
					out.close()
					return
				}
			}
			if !open {
				return
			}
		}
	}()

	var reading sync.Mutex
	buf := make([]byte, 64<<10)
	var once sync.Once
	shut := func() {
		once.Do(func() {
			out.close()
			// Close drains for up to drainTimeout, so never on the page's turn.
			go func() { defer recoverInGoroutine(); conn.Close() }()
		})
	}

	api := js.Global().Get("Object").New()
	api.Set("read", js.FuncOf(func(js.Value, []js.Value) any {
		return promise(func() (any, error) {
			reading.Lock()
			defer reading.Unlock()
			n, err := conn.Read(buf)
			if n > 0 {
				bytes := js.Global().Get("Uint8Array").New(n)
				js.CopyBytesToJS(bytes, buf[:n])
				return bytes, nil
			}
			if err == nil || errors.Is(err, io.EOF) {
				return js.Null(), nil
			}
			// A read after close, or a tunnel that dropped: the stream is over
			// either way, and the SSH layer above says which.
			return js.Null(), nil
		})
	}))
	api.Set("write", js.FuncOf(func(_ js.Value, args []js.Value) any {
		// Anything but a Uint8Array is refused rather than read: a panic in a
		// js.FuncOf callback takes the module down.
		if len(args) < 1 || !args[0].InstanceOf(js.Global().Get("Uint8Array")) {
			return nil
		}
		b := make([]byte, args[0].Get("length").Int())
		js.CopyBytesToGo(b, args[0])
		out.push(b)
		return nil
	}))
	api.Set("close", js.FuncOf(func(js.Value, []js.Value) any {
		shut()
		return nil
	}))
	return api
}

// outbox is an unbounded, ordered queue of writes with one consumer.
//
// Unbounded because the producer is a page callback that must never block;
// SSH's own channel windows are what bound how much a terminal has in flight.
type outbox struct {
	mu     sync.Mutex
	queue  [][]byte
	closed bool
	wake   chan struct{}
}

func newOutbox() *outbox { return &outbox{wake: make(chan struct{}, 1)} }

func (o *outbox) push(b []byte) {
	o.mu.Lock()
	if o.closed {
		o.mu.Unlock()
		return
	}
	o.queue = append(o.queue, b)
	o.mu.Unlock()
	o.signal()
}

func (o *outbox) close() {
	o.mu.Lock()
	o.closed = true
	o.mu.Unlock()
	o.signal()
}

func (o *outbox) signal() {
	select {
	case o.wake <- struct{}{}:
	default:
	}
}

// take waits for writes, and answers them with whether more may follow.
func (o *outbox) take() ([][]byte, bool) {
	for {
		o.mu.Lock()
		if len(o.queue) > 0 || o.closed {
			batch := o.queue
			o.queue = nil
			open := !o.closed
			o.mu.Unlock()
			return batch, open
		}
		o.mu.Unlock()
		<-o.wake
	}
}

// promise runs f on a goroutine of its own and settles a Promise with it.
func promise(f func() (any, error)) js.Value {
	var handler js.Func
	handler = js.FuncOf(func(_ js.Value, args []js.Value) any {
		resolve, reject := args[0], args[1]
		go func() {
			defer handler.Release()
			defer func() {
				if r := recover(); r != nil {
					reject.Invoke(js.Global().Get("Error").New("io"))
				}
			}()
			value, err := f()
			if err != nil {
				reject.Invoke(js.Global().Get("Error").New(err.Error()))
				return
			}
			resolve.Invoke(value)
		}()
		return nil
	})
	return js.Global().Get("Promise").New(handler)
}

func rejected(word string) js.Value {
	return js.Global().Get("Promise").Call("reject", js.Global().Get("Error").New(word))
}
