package main

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"log"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"sync/atomic"
	"syscall"
	"testing"
	"time"

	"github.com/tailscale/tailcat"
	"tailscale.com/tailcfg"
	"tailscale.com/types/key"
)

// Two node keys in the form the fence writes: 43 unpadded base64-URL
// characters.
//
// These are the constants the Rust allowlist tests use, and they now agree.
// They did not: that file's second key ended "…AAD" against this one's "…AAA",
// and 43 characters carry 258 bits, so the last character's low two bits are
// padding and those two decoded to the same 32 bytes — two devices to a test
// that compares strings, one device to the runner that decodes them. Both
// sides say "…AAE" now.
const (
	keyA = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
	keyB = "3q2-7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAE"
)

// The socketpair is the whole FFI contract: Go owns one end, Rust gets the
// other as a file descriptor, and neither needs to understand the other's
// runtime. This tests the pump in isolation, without DERP.
func TestPumpMovesBytesBothWays(t *testing.T) {
	far, near := net.Pipe() // stands in for the tailcat conn
	fd, err := pumpToSocketpair(far)
	if err != nil {
		t.Fatalf("pumpToSocketpair: %v", err)
	}
	ours, err := fdConn(fd)
	if err != nil {
		t.Fatalf("fdConn: %v", err)
	}
	defer ours.Close()

	go near.Write([]byte("down"))
	buf := make([]byte, 4)
	if _, err := ours.Read(buf); err != nil {
		t.Fatalf("read: %v", err)
	}
	if string(buf) != "down" {
		t.Fatalf("got %q, want %q", buf, "down")
	}

	ours.Write([]byte("up"))
	up := make([]byte, 2)
	if _, err := near.Read(up); err != nil {
		t.Fatalf("read up: %v", err)
	}
	if string(up) != "up" {
		t.Fatalf("got %q, want %q", up, "up")
	}
}

// watchedConn reports when the pump closed it.
type watchedConn struct {
	net.Conn
	once   sync.Once
	closed chan struct{}
}

// The channel closes AFTER the underlying Close, not before. net.Pipe hands a
// write to a blocked reader, and the pump has one blocked on the far side
// until that Close lands — so announcing the teardown first left a window in
// which the write below was accepted by a connection on its way out. It was a
// one-in-ten flake, found by running the suite in a loop rather than once.
func (w *watchedConn) Close() error {
	err := w.Conn.Close()
	w.once.Do(func() { close(w.closed) })
	return err
}

// Closing the Rust end must not leak the goroutine or the tailcat conn.
//
// The conn is watched rather than probed with a write: net.Pipe hands a write
// straight to whichever reader is blocked on it, so the pump's own copy
// goroutine accepts one write before it notices the closed descriptor, and a
// bare "the write failed" assertion passes for the wrong reason or fails for
// one.
func TestClosingOurEndTearsDownTheFarSide(t *testing.T) {
	far, near := net.Pipe()
	watched := &watchedConn{Conn: far, closed: make(chan struct{})}
	fd, err := pumpToSocketpair(watched)
	if err != nil {
		t.Fatalf("pumpToSocketpair: %v", err)
	}
	ours, _ := fdConn(fd)
	ours.Close()

	select {
	case <-watched.closed:
	case <-time.After(5 * time.Second):
		t.Fatal("closing the descriptor left the tunnel connection open")
	}
	if _, err := near.Write([]byte("x")); err == nil {
		t.Fatal("far side still accepted a write after the near side closed")
	}
}

// The refusal is checked by its exact errno rather than by "rc is negative",
// because a serve that got past this guard would go on to Start a real server
// and could fail there for an unrelated reason — leaving the test green where
// there is no network and red where there is, which is no test at all.
func TestServeRefusesAnEmptyAllowlist(t *testing.T) {
	if rc := serve(t.TempDir()+"/key", 22, ""); rc != -int(syscall.EINVAL) {
		t.Fatalf("empty allowlist: rc=%d, want %d", rc, -int(syscall.EINVAL))
	}
}

// The guard above is only half of revocation. Revoking a fleet's last device
// leaves no node keys at all, and a server that keeps running keeps admitting
// every device that was in the file when it started — tailcat copies the
// allowlist at Start and consults it only when a client first registers.
func TestServeWithAnEmptyAllowlistStopsTheRunningServer(t *testing.T) {
	running := &tailcat.Server{}
	closes := 0
	defer restoreServerState(t, running, func(s *tailcat.Server) error {
		if s == running {
			closes++
		}
		return nil
	})()

	if rc := serve(t.TempDir()+"/key", 22, ""); rc != -int(syscall.EINVAL) {
		t.Fatalf("empty allowlist: rc=%d, want %d", rc, -int(syscall.EINVAL))
	}
	if closes != 1 {
		t.Fatalf("the running server was closed %d times, want 1", closes)
	}
	if server != nil {
		t.Fatal("a server survived an allowlist that admits nobody")
	}
}

// restoreServerState installs a server and a close hook, and returns the
// function that puts the package's globals back.
func restoreServerState(t *testing.T, s *tailcat.Server, closer func(*tailcat.Server) error) func() {
	t.Helper()
	prevServer, prevCloser := server, closeServer
	server, closeServer = s, closer
	return func() { server, closeServer = prevServer, prevCloser }
}

// Tailcat reads an empty AllowedClients as "admit everyone", so the list this
// hands it is never empty: the zero node key, which nobody can hold, goes in
// first. Deleting that seed is the difference between a runner that admits two
// devices and one that admits the internet.
func TestTheAllowlistHandedToTailcatIsNeverEmpty(t *testing.T) {
	if got := allowedClients(nil); len(got) == 0 {
		t.Fatal("an empty allowlist reached tailcat, which reads it as admit-everyone")
	}
	a, err := parseNodePublic(keyA)
	if err != nil {
		t.Fatalf("parseNodePublic: %v", err)
	}
	got := allowedClients([]key.NodePublic{a})
	if !slices.Contains(got, key.NodePublic{}) {
		t.Fatal("the unholdable seed key is missing from the allowlist")
	}
	if !slices.Contains(got, a) {
		t.Fatal("the device's own node key is missing from the allowlist")
	}
}

func TestParseAllowedReadsTheKeysTheFenceWrites(t *testing.T) {
	got, err := parseAllowed(keyA + "\n" + keyB + "\n")
	if err != nil {
		t.Fatalf("parseAllowed: %v", err)
	}
	if len(got) != 2 {
		t.Fatalf("got %d keys, want 2", len(got))
	}
	if got[0] == got[1] {
		t.Fatal("two different node keys decoded to the same key")
	}
}

func TestParseAllowedRefusesAKeyItCannotRead(t *testing.T) {
	for _, bad := range []string{
		"tooshort",
		keyA + "x", // 44 characters
		"3q2+7wAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", // standard base64, not URL
		"nodekey:" + keyA, // tailscale's own text form
	} {
		if _, err := parseAllowed(bad); err == nil {
			t.Fatalf("%q was accepted as a node key", bad)
		}
	}
}

// A device's private key is generated outside Go and arrives as raw bytes,
// while the public key its owner published was derived by an X25519
// implementation that clamps. The two must agree, or the runner computes a
// different public key and silently ignores the device — silently, because an
// unrecognized client gets no refusal, only a timeout.
//
// This is a property of the dependency rather than of the code above: x/crypto
// clamps inside ScalarMult, so both spellings of the same key derive the same
// public key. It is here because if that ever stops being true, this is the
// only place that would notice, and the symptom otherwise is a device that
// times out for no stated reason.
func TestAnUnclampedPrivateKeyDerivesTheClampedPublicKey(t *testing.T) {
	raw := make([]byte, 32)
	for i := range raw {
		raw[i] = 0xff
	}
	loose, err := parseNodePrivate(base64.RawURLEncoding.EncodeToString(raw))
	if err != nil {
		t.Fatalf("parseNodePrivate: %v", err)
	}
	raw[0] &= 248
	raw[31] &= 127
	raw[31] |= 64
	tight, err := parseNodePrivate(base64.RawURLEncoding.EncodeToString(raw))
	if err != nil {
		t.Fatalf("parseNodePrivate: %v", err)
	}
	if loose.Public() != tight.Public() {
		t.Fatal("an unclamped private key derived a different public key than the clamped one")
	}
}

func TestParseNodePrivateAcceptsTailscalesOwnForm(t *testing.T) {
	generated := key.NewNode()
	text, err := generated.MarshalText()
	if err != nil {
		t.Fatalf("MarshalText: %v", err)
	}
	parsed, err := parseNodePrivate(string(text))
	if err != nil {
		t.Fatalf("parseNodePrivate: %v", err)
	}
	if parsed.Public() != generated.Public() {
		t.Fatal("a round trip through tailscale's own key form changed the key")
	}
}

func TestTheIdentityFileIsCreatedUnreadableByAnybodyElse(t *testing.T) {
	path := filepath.Join(t.TempDir(), "node.key")
	first, err := loadOrCreateIdentity(path)
	if err != nil {
		t.Fatalf("loadOrCreateIdentity: %v", err)
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatalf("stat: %v", err)
	}
	if perm := info.Mode().Perm(); perm != 0o600 {
		t.Fatalf("identity file is mode %04o, want 0600", perm)
	}
	// Its existence is the feature flag, so it has to survive a restart: a
	// runner that generated a new identity on every start would hand out a
	// token that stopped working the moment the daemon restarted.
	second, err := loadOrCreateIdentity(path)
	if err != nil {
		t.Fatalf("second loadOrCreateIdentity: %v", err)
	}
	if !first.priv.Equal(second.priv) {
		t.Fatal("the runner's identity changed between two reads of the same file")
	}
	// A fresh identity names no region: which relay this runner listens on is
	// not known until Start has picked one.
	if first.regionID != 0 {
		t.Fatalf("a fresh identity claimed region %d", first.regionID)
	}
}

// The token a device holds names a DERP region. Upstream picks the nearest one
// by latency whenever RegionID is zero, so a runner that did not remember its
// choice would pick again after a restart on a different network — and every
// token already in a device's manifest would name a relay this runner no
// longer listens on, with nothing said to anybody.
func TestTheChosenRegionSurvivesARestart(t *testing.T) {
	path := filepath.Join(t.TempDir(), "node.key")
	first, err := loadOrCreateIdentity(path)
	if err != nil {
		t.Fatalf("loadOrCreateIdentity: %v", err)
	}
	if err := pinRegion(path, identity{priv: first.priv, regionID: 302}); err != nil {
		t.Fatalf("pinRegion: %v", err)
	}
	again, err := loadOrCreateIdentity(path)
	if err != nil {
		t.Fatalf("loadOrCreateIdentity after pinning: %v", err)
	}
	if again.regionID != 302 {
		t.Fatalf("region came back as %d, want 302", again.regionID)
	}
	if !again.priv.Equal(first.priv) {
		t.Fatal("pinning the region changed the runner's key")
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatalf("stat: %v", err)
	}
	if perm := info.Mode().Perm(); perm != 0o600 {
		t.Fatalf("identity file is mode %04o after pinning, want 0600", perm)
	}
	if _, err := os.Stat(path + ".new"); !os.IsNotExist(err) {
		t.Fatal("pinRegion left its temporary file behind")
	}
}

// A file written before a region was ever pinned is one line, and still has to
// load.
func TestAnIdentityFileWithNoRegionStillLoads(t *testing.T) {
	path := filepath.Join(t.TempDir(), "node.key")
	priv := key.NewNode()
	text, err := priv.MarshalText()
	if err != nil {
		t.Fatalf("MarshalText: %v", err)
	}
	if err := os.WriteFile(path, append(text, '\n'), 0o600); err != nil {
		t.Fatalf("write: %v", err)
	}
	id, err := loadOrCreateIdentity(path)
	if err != nil {
		t.Fatalf("loadOrCreateIdentity: %v", err)
	}
	if !id.priv.Equal(priv) || id.regionID != 0 {
		t.Fatalf("got region %d and a %s key", id.regionID, map[bool]string{true: "matching", false: "different"}[id.priv.Equal(priv)])
	}
}

// A node key anyone on the runner can read is a route anyone on the runner can
// take. This is the posture the fence takes about authorized_keys.
func TestAGroupReadableKeyFileIsRefused(t *testing.T) {
	path := filepath.Join(t.TempDir(), "node.key")
	if _, err := loadOrCreateIdentity(path); err != nil {
		t.Fatalf("loadOrCreateIdentity: %v", err)
	}
	if err := os.Chmod(path, 0o640); err != nil {
		t.Fatalf("chmod: %v", err)
	}
	if _, err := loadOrCreateIdentity(path); err == nil {
		t.Fatal("a group-readable node key file was read anyway")
	}
	// And through the entry point the daemon actually calls, which proves the
	// allowlist guard ran first and this one second.
	if rc := serve(path, 22, keyA); rc != -int(syscall.EACCES) {
		t.Fatalf("serve on a group-readable key file: rc=%d, want %d", rc, -int(syscall.EACCES))
	}
}

// The data plane is chatty enough to bury the daemon's own log. What is
// dropped is only what tailscale itself marks as verbose; a line that explains
// a failure still arrives.
func TestVerboseTailcatLinesAreDroppedAndTheRestKept(t *testing.T) {
	var out strings.Builder
	prev := log.Writer()
	log.SetOutput(&out)
	log.SetFlags(0)
	defer func() { log.SetOutput(prev); log.SetFlags(log.LstdFlags) }()

	tailcatLogf("[v2] netstack: registered IP %s", "::/0")
	tailcatLogf("wg: [v2] Routine: receive incoming %s - started", "mkReceiveFunc")
	tailcatLogf("netcheck: [v1] report: %s", "udp=false")
	tailcatLogf("magicsock: %s", "could not reach the relay")

	got := out.String()
	for _, verbose := range []string{"netstack", "mkReceiveFunc", "udp=false"} {
		if strings.Contains(got, verbose) {
			t.Fatalf("a verbose line reached the log: %q", got)
		}
	}
	if !strings.Contains(got, "tailcat: magicsock: could not reach the relay") {
		t.Fatalf("the line that explains a failure was dropped: %q", got)
	}
}

func TestConnBlobWithoutARunningServerIsRefused(t *testing.T) {
	defer restoreServerState(t, nil, closeServer)()
	if rc := connBlob(make([]byte, 256)); rc != -int(syscall.ENOTCONN) {
		t.Fatalf("connBlob with no server: rc=%d, want %d", rc, -int(syscall.ENOTCONN))
	}
}

func TestAllowAddWithoutARunningServerIsRefused(t *testing.T) {
	defer restoreServerState(t, nil, closeServer)()
	if rc := allowAdd(keyA); rc != -int(syscall.ENOTCONN) {
		t.Fatalf("allowAdd with no server: rc=%d, want %d", rc, -int(syscall.ENOTCONN))
	}
}

// The seed key is not a device. Admitting it would report success for a
// registration that grants nothing, and it is the one key that must never be
// addable by name.
func TestAllowAddRefusesTheSeedKey(t *testing.T) {
	zero := base64.RawURLEncoding.EncodeToString(make([]byte, 32))
	if rc := allowAdd(zero); rc != -int(syscall.EINVAL) {
		t.Fatalf("allowAdd of the zero key: rc=%d, want %d", rc, -int(syscall.EINVAL))
	}
}

// A dial that cannot read its arguments must fail before it touches the
// network, or the caller waits out a DERP timeout to be told it typed a key
// wrong.
func TestDialRefusesArgumentsItCannotRead(t *testing.T) {
	if rc := dial("", keyA, 22); rc != -int(syscall.EINVAL) {
		t.Fatalf("dial with no token: rc=%d, want %d", rc, -int(syscall.EINVAL))
	}
	// A token good enough to get past the parse, so this tests the key.
	token := mustFakeToken(t, "127.0.0.1", closedPort(t))
	if rc := dial(token, "not-a-node-key", 22); rc != -int(syscall.EINVAL) {
		t.Fatalf("dial with an unreadable client key: rc=%d, want %d", rc, -int(syscall.EINVAL))
	}
}

// closedPort returns a loopback port nothing is listening on.
func closedPort(t *testing.T) int {
	t.Helper()
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	port := listener.Addr().(*net.TCPAddr).Port
	listener.Close()
	return port
}

// A zero private key is not an error to tailcat: it means "generate an
// ephemeral one". A device that dialed with it would hold an identity that is
// in no allowlist, be ignored without a word, and spend the whole ten-second
// handshake arriving at a timeout that names the runner rather than the key.
func TestDialRefusesAZeroPrivateKey(t *testing.T) {
	captureLog(t)
	zero := base64.RawURLEncoding.EncodeToString(make([]byte, 32))
	// The token has to be one this would otherwise use, or the refusal below
	// could be the token's and the guard would be untested.
	token := mustFakeToken(t, "127.0.0.1", closedPort(t))
	done := make(chan int, 1)
	go func() { done <- dial(token, zero, 22) }()
	select {
	case rc := <-done:
		if rc != -int(syscall.EINVAL) {
			t.Fatalf("dial with a zero private key: rc=%d, want %d", rc, -int(syscall.EINVAL))
		}
	case <-time.After(5 * time.Second):
		t.Fatal("dial with a zero private key went to the network instead of refusing")
	}
}

// A token that is not a token is the caller's mistake, and saying "can't reach
// the rendezvous service" about it blames the network for a typo. It is also
// what enforces the invariant the attribution below rests on: our tokens
// always embed their region, so the handshake's first leg never goes to the
// network.
func TestDialRefusesATokenItCannotUse(t *testing.T) {
	captureLog(t)
	priv := newPrivateKeyText(t)
	for _, bad := range []string{"tcnotarealtoken", "not a token at all", string(mustFakeToken(t, "", 0))} {
		if rc := dial(bad, priv, 22); rc != -int(syscall.EINVAL) {
			t.Fatalf("dial on %q: rc=%d, want %d", bad, rc, -int(syscall.EINVAL))
		}
	}
}

// newPrivateKeyText returns a usable client key in tailscale's own text form.
func newPrivateKeyText(t *testing.T) string {
	t.Helper()
	text, err := key.NewNode().MarshalText()
	if err != nil {
		t.Fatalf("MarshalText: %v", err)
	}
	return string(text)
}

// mustFakeToken builds a token naming one DERP node at host:port, or, with an
// empty host, one that embeds no region at all.
func mustFakeToken(t *testing.T, host string, port int) string {
	t.Helper()
	srv := key.NewNode()
	ci := tailcat.ConnInfo{
		ServerPublic:      tailcat.NodePublic{NodePublic: srv.Public()},
		ServerDiscoPublic: tailcat.DiscoPublicForNode(srv),
	}
	if host != "" {
		ci.Region = []*tailcfg.DERPRegion{{
			RegionID:   900,
			RegionCode: "test",
			RegionName: "test",
			Nodes: []*tailcfg.DERPNode{{
				Name:             "900a",
				RegionID:         900,
				HostName:         host,
				IPv4:             host,
				DERPPort:         port,
				STUNPort:         -1,
				InsecureForTests: true,
			}},
		}}
	}
	return string(ci.ConnBlob())
}

// The probe that makes "Can't reach the rendezvous service" a sentence that
// can actually be shown. Upstream's meow is fire-and-forget — magicsock
// answers "sent" as soon as the packet lands on a channel — so a dead relay
// and a silent runner look identical without it.
func TestARelayIsProbedForReachability(t *testing.T) {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen: %v", err)
	}
	defer listener.Close()
	up := listener.Addr().(*net.TCPAddr).Port

	live, err := tailcat.ParseConnBlob(tailcat.ConnBlob(mustFakeToken(t, "127.0.0.1", up)))
	if err != nil {
		t.Fatalf("ParseConnBlob: %v", err)
	}
	if !relayReachable(live) {
		t.Fatal("a relay that is listening was reported unreachable")
	}

	listener.Close()
	dead, err := tailcat.ParseConnBlob(tailcat.ConnBlob(mustFakeToken(t, "127.0.0.1", up)))
	if err != nil {
		t.Fatalf("ParseConnBlob: %v", err)
	}
	if relayReachable(dead) {
		t.Fatal("a relay that is not listening was reported reachable")
	}
}

// lockedBuffer collects log output written from more than one goroutine.
type lockedBuffer struct {
	mu  sync.Mutex
	buf strings.Builder
}

func (b *lockedBuffer) Write(p []byte) (int, error) {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.Write(p)
}

func (b *lockedBuffer) String() string {
	b.mu.Lock()
	defer b.mu.Unlock()
	return b.buf.String()
}

// captureLog sends the standard logger into a buffer for the length of a test.
func captureLog(t *testing.T) *lockedBuffer {
	t.Helper()
	var out lockedBuffer
	prev := log.Writer()
	log.SetOutput(&out)
	log.SetFlags(0)
	t.Cleanup(func() { log.SetOutput(prev); log.SetFlags(log.LstdFlags) })
	return &out
}

// Upstream's connBlob panics on "no DERPMap set" and "no regions in derpmap",
// and it is reached synchronously from fc_tailcat_conn_blob. Without a barrier
// that panic aborts farcoolerd, or the app on a phone. A server that was never
// started stands in for it: the same call, the same crash, one line earlier.
func TestAPanicUnderConnBlobBecomesAnErrnoRatherThanACrash(t *testing.T) {
	captureLog(t)
	defer restoreServerState(t, &tailcat.Server{}, func(*tailcat.Server) error { return nil })()
	if rc := connBlob(make([]byte, 512)); rc != -int(syscall.EIO) {
		t.Fatalf("connBlob over a panicking server: rc=%d, want %d", rc, -int(syscall.EIO))
	}
}

// panickyConn stands in for Client.DrainTCP, whose reflection into netstack's
// unexported ipstack field upstream says panics on purpose if the tailscale
// dependency drifts.
type panickyConn struct {
	net.Conn
}

func (p *panickyConn) Close() error {
	panic("tcpipStackOf: netstack.Impl has no ipstack field")
}

// The teardown runs in a goroutine this package started, and a panic in a
// goroutine takes the whole process with it however far up the stack the
// caller is. This test would not fail if the barrier were missing — it would
// abort the test binary, which is the same thing said louder.
func TestAPanicWhileTearingDownDoesNotTakeTheProcess(t *testing.T) {
	out := captureLog(t)
	far, _ := net.Pipe()
	fd, err := pumpToSocketpair(&panickyConn{Conn: far})
	if err != nil {
		t.Fatalf("pumpToSocketpair: %v", err)
	}
	ours, err := fdConn(fd)
	if err != nil {
		t.Fatalf("fdConn: %v", err)
	}
	ours.Close()

	deadline := time.Now().Add(5 * time.Second)
	for !strings.Contains(out.String(), "recovered from a panic while tearing a tunnel down") {
		if time.Now().After(deadline) {
			t.Fatalf("the teardown panic was never recovered; log was %q", out.String())
		}
		time.Sleep(10 * time.Millisecond)
	}
}

// What a device's manifest actually has to carry, measured rather than
// estimated, and the pin proved against a real Start. It starts real servers
// against a public DERP relay, so it is opt-in: the rest of this package's
// tests touch no network at all.
func TestAProductionTokenIsMeasuredAndDoesNotMoveAcrossARestart(t *testing.T) {
	if os.Getenv("FARCOOLER_TAILCAT_LIVE") == "" {
		t.Skip("starts a real server against a public DERP relay; set FARCOOLER_TAILCAT_LIVE=1 to run it")
	}
	path := filepath.Join(t.TempDir(), "node.key")
	t.Cleanup(func() { mu.Lock(); stopServer(); mu.Unlock() })

	token, relay := startAndReadToken(t, path)
	t.Logf("a production ConnBlob is %d bytes", len(token))
	t.Logf("token: %s", token)
	t.Logf("relay: %s", relay)

	id, err := loadOrCreateIdentity(path)
	if err != nil {
		t.Fatalf("loadOrCreateIdentity: %v", err)
	}
	if id.regionID == 0 {
		t.Fatal("a region was chosen and not pinned; the next restart would choose again")
	}
	t.Logf("pinned region: %d", id.regionID)

	// The restart is the whole point of the pin: the token already in a
	// device's manifest has to keep naming a relay this runner listens on.
	mu.Lock()
	stopServer()
	mu.Unlock()
	again, relayAgain := startAndReadToken(t, path)
	if relayAgain != relay {
		t.Fatalf("the runner moved relays across a restart: %s then %s", relay, relayAgain)
	}
	if again != token {
		t.Logf("the token changed across the restart but the relay did not: %d bytes now", len(again))
	}
}

// startAndReadToken serves, then returns the runner's token and the DERP node
// that token names.
func startAndReadToken(t *testing.T, path string) (token, relay string) {
	t.Helper()
	if rc := serve(path, 22, keyA); rc != 0 {
		t.Fatalf("serve: rc=%d", rc)
	}
	buf := make([]byte, 8192)
	n := connBlob(buf)
	if n < 0 {
		t.Fatalf("connBlob: rc=%d", n)
	}
	token = string(buf[:n])
	ci, err := tailcat.ParseConnBlob(tailcat.ConnBlob(token))
	if err != nil {
		t.Fatalf("the runner's own token does not parse: %v", err)
	}
	if len(ci.Region) == 0 || len(ci.Region[0].Nodes) == 0 {
		t.Fatal("the token embeds no DERP node, so a client would have to fetch the map")
	}
	return token, ci.Region[0].Nodes[0].HostName
}

// The other leg, which needs a relay: a device whose node key is not in the
// runner's allowlist is added as no peer and told nothing, so the handshake
// simply runs out. That must be reported as the runner rather than as the
// rendezvous — this is the revoked device, and "This runner didn't answer, its
// access may have been revoked" is the sentence the spec owes it.
func TestADeviceOutsideTheAllowlistIsBlamedOnTheRunnerNotTheRelay(t *testing.T) {
	if os.Getenv("FARCOOLER_TAILCAT_LIVE") == "" {
		t.Skip("needs a real DERP relay; set FARCOOLER_TAILCAT_LIVE=1 to run it")
	}
	path := filepath.Join(t.TempDir(), "node.key")
	t.Cleanup(func() { mu.Lock(); stopServer(); mu.Unlock() })
	token, _ := startAndReadToken(t, path) // admits keyA and nobody else

	stranger, err := key.NewNode().MarshalText()
	if err != nil {
		t.Fatalf("MarshalText: %v", err)
	}
	start := time.Now()
	rc := dial(token, string(stranger), 22)
	t.Logf("a stranger's dial answered %d after %s", rc, time.Since(start).Round(time.Millisecond))
	if rc != -int(syscall.ETIMEDOUT) {
		t.Fatalf("a device outside the allowlist: rc=%d, want %d", rc, -int(syscall.ETIMEDOUT))
	}
}

// The dial-level half of the probe, and the sentence it earns. A token whose
// relay is not there must blame the rendezvous, not the runner — the runner is
// blameless and may not even exist. It costs upstream's ten-second handshake
// timer, which is not ours to shorten.
func TestATokenWhoseRelayIsGoneBlamesTheRendezvous(t *testing.T) {
	captureLog(t)
	token := mustFakeToken(t, "127.0.0.1", closedPort(t))
	start := time.Now()
	rc := dial(token, newPrivateKeyText(t), 22)
	t.Logf("a dead relay answered %d after %s", rc, time.Since(start).Round(time.Millisecond))
	if rc != -int(syscall.EHOSTUNREACH) {
		t.Fatalf("dial through a relay that is gone: rc=%d, want %d", rc, -int(syscall.EHOSTUNREACH))
	}
}

// serveDERPMap stands up a DERP map server listing exactly the regions given,
// and points this package at it for the length of the test.
//
// The nodes are loopback with STUN disabled, so netcheck fails against them
// fast rather than probing the internet: nothing here is meant to carry a
// packet, only to be a map that either does or does not list a region.
func serveDERPMap(t *testing.T, regionIDs ...int) *atomic.Int64 {
	t.Helper()
	regions := map[string]any{}
	for _, id := range regionIDs {
		regions[fmt.Sprint(id)] = map[string]any{
			"RegionID": id, "RegionCode": fmt.Sprint(id), "RegionName": fmt.Sprint(id),
			"Nodes": []any{map[string]any{
				"Name": fmt.Sprintf("%da", id), "RegionID": id,
				"HostName": "127.0.0.1", "IPv4": "127.0.0.1",
				"DERPPort": closedPort(t), "STUNPort": -1,
				"InsecureForTests": true,
			}},
		}
	}
	body, err := json.Marshal(map[string]any{"Regions": regions})
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	var fetches atomic.Int64
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		fetches.Add(1)
		w.Header().Set("Content-Type", "application/json")
		w.Write(body)
	}))
	t.Cleanup(server.Close)
	pointAtDERPMap(t, server.URL+"/derpmap.json")
	return &fetches
}

// pointAtDERPMap sets the package's DERP map URL for the length of the test.
// Each caller gets a distinct URL, because upstream caches a fetched map
// process-wide for an hour and a shared URL would leak one test's map into the
// next.
func pointAtDERPMap(t *testing.T, url string) {
	t.Helper()
	mu.Lock()
	prev := derpMapURL
	derpMapURL = url
	mu.Unlock()
	t.Cleanup(func() { mu.Lock(); derpMapURL = prev; mu.Unlock() })
}

// pinnedRegion reads the region an identity file currently names.
func pinnedRegion(t *testing.T, path string) tailcfg.DERPRegionID {
	t.Helper()
	id, err := loadOrCreateIdentity(path)
	if err != nil {
		t.Fatalf("loadOrCreateIdentity: %v", err)
	}
	return id.regionID
}

// pinnedIdentity creates an identity file already naming a region.
func pinnedIdentity(t *testing.T, regionID tailcfg.DERPRegionID) string {
	t.Helper()
	path := filepath.Join(t.TempDir(), "node.key")
	id, err := loadOrCreateIdentity(path)
	if err != nil {
		t.Fatalf("loadOrCreateIdentity: %v", err)
	}
	if err := pinRegion(path, identity{priv: id.priv, regionID: regionID}); err != nil {
		t.Fatalf("pinRegion: %v", err)
	}
	return path
}

// A pinned region that has genuinely left the DERP map is fatal to Start, and
// the pin is on disk — so a runner that did not forget it would fail
// identically on every start until somebody edited the file. It used to
// re-pick and serve; that is not a regression this is willing to ship.
//
// The recovery finishes inside this one serve: a replacement is chosen from
// the same map, written down, and started, rather than leaving the runner up
// with a token that would move again next time.
func TestARegionThatHasLeftTheMapIsForgotten(t *testing.T) {
	captureLog(t)
	defer restoreServerState(t, nil, closeServer)()
	fetches := serveDERPMap(t, 901) // 900 is not in it
	path := pinnedIdentity(t, 900)

	start := time.Now()
	rc := serve(path, 22, keyA)
	elapsed := time.Since(start)
	t.Logf("recovering from a withdrawn region took %s and %d map fetches", elapsed.Round(time.Millisecond), fetches.Load())
	if rc != 0 {
		t.Fatalf("serve did not recover from a withdrawn region: rc=%d", rc)
	}
	// The worst path reads the map twice by the code's shape -- once inside
	// Start, once to ask whether the region is still listed -- and serve holds
	// mu for both. Upstream caches a fetched map process-wide for an hour, so
	// the second read should cost no request at all. That is the difference
	// between one regionPickTimeout under the lock and two, and it is asserted
	// here rather than assumed.
	if got := fetches.Load(); got != 1 {
		t.Fatalf("the DERP map was fetched %d times; the process-wide cache did not collapse the second read", got)
	}
	if server == nil {
		t.Fatal("the retry succeeded and its server was not recorded")
	}
	got := pinnedRegion(t, path)
	t.Logf("region 900 left the map; the runner is now pinned to %d", got)
	if got == 900 {
		t.Fatal("the runner kept region 900 after it left the map; every restart fails the same way")
	}
}

// The other arm, and the one that matters more, because it is the common case.
// Start fetches the DERP map before it does anything else, so its commonest
// failure is a map it could not read — a runner serving at boot before the
// network is up, or a laptop resuming from sleep. Forgetting a good pin over
// one of those trades a brick for a token that silently moves, which is the
// harm the pin exists to prevent.
func TestATransientlyUnreachableMapKeepsThePin(t *testing.T) {
	captureLog(t)
	defer restoreServerState(t, nil, closeServer)()
	pointAtDERPMap(t, fmt.Sprintf("http://127.0.0.1:%d/derpmap.json", closedPort(t)))
	path := pinnedIdentity(t, 900)

	if rc := serve(path, 22, keyA); rc != -int(syscall.EIO) {
		t.Fatalf("serve with an unreachable map: rc=%d, want %d", rc, -int(syscall.EIO))
	}
	if got := pinnedRegion(t, path); got != 900 {
		t.Fatalf("a network blip erased the pin (region is now %d); every token in a manifest may now be stale", got)
	}
	if server != nil {
		t.Fatal("a server was recorded although none started")
	}
}

// And the third: the map reads fine and still lists the region, so whatever
// stopped Start was not the region going away.
//
// This calls the decision directly, because there is no way to make Start fail
// for an unrelated reason while its region is present — and a test that let
// Start succeed would assert nothing at all.
func TestARegionStillInTheMapKeepsThePin(t *testing.T) {
	captureLog(t)
	defer restoreServerState(t, nil, closeServer)()
	serveDERPMap(t, 900)
	path := pinnedIdentity(t, 900)
	id, err := loadOrCreateIdentity(path)
	if err != nil {
		t.Fatalf("loadOrCreateIdentity: %v", err)
	}
	allowed, err := parseAllowed(keyA)
	if err != nil {
		t.Fatalf("parseAllowed: %v", err)
	}

	// serve holds mu across this call; nothing here is driven concurrently, so
	// it is called bare. A test that ever drives two of these at once has to
	// take mu, because the recovery reads derpMapURL and writes server.
	var s *tailcat.Server
	rc := recoverFromAPinThatWillNotStart(&s, path, id, 22, allowed, errors.New("something else went wrong"))
	if rc != -int(syscall.EIO) {
		t.Fatalf("a region still in the map: rc=%d, want %d", rc, -int(syscall.EIO))
	}
	if got := pinnedRegion(t, path); got != 900 {
		t.Fatalf("region 900 is still in the map and the pin was dropped anyway (region is now %d)", got)
	}
	if s != nil {
		t.Fatal("a server was built for a region that never went away")
	}
}

// Where the withdrawn-region path actually spends its time, and which half of
// it this package bounds.
//
// The comment on serve used to say "two Starts plus one regionPickTimeout",
// which holds only when the recovery's pick succeeds. When it returns 0 the
// replacement Server carries RegionID 0 — and to upstream that is not "no
// region", it is cmp.Or(0, -1) = -1, "choose one for me", whose netcheck runs
// inside Expand on context.Background() under a hardcoded ten seconds that
// nothing here bounds.
//
// So the path is two netchecks and only one of them is ours. This measures all
// three numbers in one run so the attribution is an assertion rather than
// arithmetic across two test runs.
func TestTheWithdrawnRegionPathIsTwoPicksAndOnlyOneIsOurs(t *testing.T) {
	captureLog(t)
	defer restoreServerState(t, nil, closeServer)()
	serveDERPMap(t, 901) // 900 is not in it
	allowed, err := parseAllowed(keyA)
	if err != nil {
		t.Fatalf("parseAllowed: %v", err)
	}

	// The whole path, as serve pays for it with mu held.
	beforeTotal := time.Now()
	if rc := serve(pinnedIdentity(t, 900), 22, keyA); rc != 0 {
		t.Fatalf("serve did not recover from a withdrawn region: rc=%d", rc)
	}
	total := time.Since(beforeTotal)

	// Segment one: the pick this package bounds with regionPickTimeout.
	ctx, cancel := context.WithTimeout(context.Background(), regionPickTimeout)
	dm, err := regionMap(ctx, derpMapURL)
	if err != nil {
		t.Fatalf("regionMap: %v", err)
	}
	beforeOurs := time.Now()
	picked, pickErr := pickRegionIn(ctx, dm)
	ours := time.Since(beforeOurs)
	cancel()

	// Segment two: what a Start with no region of its own costs, which is the
	// second netcheck and the one on upstream's clock.
	id, err := loadOrCreateIdentity(filepath.Join(t.TempDir(), "node.key"))
	if err != nil {
		t.Fatalf("loadOrCreateIdentity: %v", err)
	}
	unpinned := newServer(identity{priv: id.priv}, 22, allowed)
	beforeTheirs := time.Now()
	startErr := unpinned.Start()
	theirs := time.Since(beforeTheirs)
	unpinned.Close()

	t.Logf("the whole withdrawn-region path: %s", total.Round(time.Millisecond))
	t.Logf("  our bounded pick:                     %s (region %d, err %v)", ours.Round(time.Millisecond), picked, pickErr)
	t.Logf("  upstream's pick inside a Start at 0:  %s (err %v)", theirs.Round(time.Millisecond), startErr)

	// Both halves have to be real, or the comment on serve is describing a
	// cost that is not there. The floor is a tenth of what either measured, so
	// this fails on a change of shape rather than on a slow afternoon.
	if ours < 500*time.Millisecond {
		t.Fatalf("our own region pick cost %s; serve's bound describes a netcheck that is no longer happening", ours)
	}
	if theirs < 500*time.Millisecond {
		t.Fatalf("a Start with RegionID 0 cost %s; upstream no longer auto-picks, and serve's bound overstates the cost", theirs)
	}
	// And they have to be most of it, or the attribution names the wrong
	// thing.
	if accounted := ours + theirs; accounted*4 < total*3 {
		t.Fatalf("two netchecks account for only %s of a %s path; the time is going somewhere unnamed", accounted, total)
	}
}
