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

func (w *watchedConn) Close() error {
	w.once.Do(func() { close(w.closed) })
	return w.Conn.Close()
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

func TestTheKeyFileIsCreatedUnreadableByAnybodyElse(t *testing.T) {
	path := filepath.Join(t.TempDir(), "node.key")
	first, err := loadOrCreateKey(path)
	if err != nil {
		t.Fatalf("loadOrCreateKey: %v", err)
	}
	info, err := os.Stat(path)
	if err != nil {
		t.Fatalf("stat: %v", err)
	}
	if perm := info.Mode().Perm(); perm != 0o600 {
		t.Fatalf("key file is mode %04o, want 0600", perm)
	}
	// Its existence is the feature flag, so it has to survive a restart: a
	// runner that generated a new identity on every start would hand out a
	// token that stopped working the moment the daemon restarted.
	second, err := loadOrCreateKey(path)
	if err != nil {
		t.Fatalf("second loadOrCreateKey: %v", err)
	}
	if !first.Equal(second) {
		t.Fatal("the runner's identity changed between two reads of the same file")
	}
}

// A node key anyone on the runner can read is a route anyone on the runner can
// take. This is the posture the fence takes about authorized_keys.
func TestAGroupReadableKeyFileIsRefused(t *testing.T) {
	path := filepath.Join(t.TempDir(), "node.key")
	if _, err := loadOrCreateKey(path); err != nil {
		t.Fatalf("loadOrCreateKey: %v", err)
	}
	if err := os.Chmod(path, 0o640); err != nil {
		t.Fatalf("chmod: %v", err)
	}
	if _, err := loadOrCreateKey(path); err == nil {
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
	tailcatLogf("magicsock: %s", "could not reach the relay")

	got := out.String()
	if strings.Contains(got, "netstack") {
		t.Fatalf("a verbose line reached the log: %q", got)
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
