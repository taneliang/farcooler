package main

import (
	"encoding/base64"
	"log"
	"net"
	"os"
	"path/filepath"
	"slices"
	"strings"
	"sync"
	"syscall"
	"testing"
	"time"

	"github.com/tailscale/tailcat"
	"tailscale.com/types/key"
)

// Two node keys in the form the fence writes: 43 unpadded base64-URL
// characters.
//
// keyA is the constant the Rust allowlist tests use. keyB is NOT theirs: their
// second constant ends "…AAB" against this one's "…AAA", and 43 characters
// carry 258 bits, so those two differ only in padding and decode to the same
// 32 bytes. Two node keys that a Rust test can tell apart by string are one
// key here, which is exactly the kind of thing a test should not be quiet
// about.
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
	if rc := dial("tcsomething", "not-a-node-key", 22); rc != -int(syscall.EINVAL) {
		t.Fatalf("dial with an unreadable client key: rc=%d, want %d", rc, -int(syscall.EINVAL))
	}
}

// A zero private key is not an error to tailcat: it means "generate an
// ephemeral one". A device that dialed with it would hold an identity that is
// in no allowlist, be ignored without a word, and spend the whole ten-second
// handshake arriving at a timeout that names the runner rather than the key.
func TestDialRefusesAZeroPrivateKey(t *testing.T) {
	zero := base64.RawURLEncoding.EncodeToString(make([]byte, 32))
	done := make(chan int, 1)
	go func() { done <- dial("tcsomething", zero, 22) }()
	select {
	case rc := <-done:
		if rc != -int(syscall.EINVAL) {
			t.Fatalf("dial with a zero private key: rc=%d, want %d", rc, -int(syscall.EINVAL))
		}
	case <-time.After(5 * time.Second):
		t.Fatal("dial with a zero private key went to the network instead of refusing")
	}
}

// A revoked device is ignored in silence, so the only thing separating "the
// relay is unreachable" from "the runner did not answer" is which leg of the
// handshake failed. This is the relay leg, and it needs no network: a token
// that names no relay fails inside the client's own startup, before any meow
// is sent.
func TestATokenThatNamesNoRelayBlamesTheRendezvous(t *testing.T) {
	captureLog(t)
	priv, err := key.NewNode().MarshalText()
	if err != nil {
		t.Fatalf("MarshalText: %v", err)
	}
	if rc := dial("tcnotarealtoken", string(priv), 22); rc != -int(syscall.EHOSTUNREACH) {
		t.Fatalf("dial on an unreadable token: rc=%d, want %d", rc, -int(syscall.EHOSTUNREACH))
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
