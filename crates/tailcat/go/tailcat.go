// Package main is the C archive the Rust side links against.
//
// It owns exactly one thing: turning a tailcat connection into a file
// descriptor. A file descriptor is not addressable, which is why this is a
// socketpair and not a loopback listener — on iOS a loopback port is reachable
// by every other app on the phone.
//
// Nothing here authenticates anybody. SSH still authenticates, authorized_keys
// still forces the command and the scope, and the fence still owns that file.
// A device that reaches a runner through this tunnel has gained a route, never
// a permission.
//
// No message from this file reaches a screen. Every entry point answers with a
// negative errno and the Rust side turns that into a stable word the apps own.
package main

/*
#include <stdint.h>
*/
import "C"

import (
	"context"
	"encoding/base64"
	"errors"
	"fmt"
	"io"
	"log"
	"net"
	"os"
	"strconv"
	"strings"
	"sync"
	"syscall"
	"time"
	"unsafe"

	"go4.org/mem"

	"github.com/tailscale/tailcat"
	"tailscale.com/types/key"
	"tailscale.com/wgengine/filter"
)

// nodeKeyLen is the length of a node key as this product writes it: the raw 32
// bytes of an X25519 public key in unpadded base64-URL.
//
// This encoding is ours, not tailcat's. Tailscale's own key.NodePublic
// marshals as "nodekey:" plus 64 hex characters; the 43-character field is the
// one that rides in an authorized_keys line, where "/" and "+" would be noise
// and "-"/"_" cannot be confused with the line's own separators.
// fence::usable_node_key enforces exactly this alphabet on the Rust side, so
// the conversion to tailscale's form happens here and nowhere else.
const nodeKeyLen = 43

// tunnelPort is the port a client asks for, and it is a name rather than an
// address. Where this runner's sshd actually listens is a fact only this
// runner has, and it is the sshPort argument to serve.
const tunnelPort = 22

// drainTimeout bounds the wait for a closed connection's last bytes. The whole
// TCP stack runs in this process, so exiting straight after a Close can lose
// the FIN; a peer that is already gone must not make us wait forever for it.
const drainTimeout = 2 * time.Second

// One server per process. A daemon has one runner and one sshd to front.
var (
	mu     sync.Mutex
	server *tailcat.Server
	// derpMapURL is empty by default, meaning tailcat's own default map. It
	// exists so that recovering from a revoked relay is a setting rather than
	// shipping three apps and every runner.
	derpMapURL string
)

// closeServer closes a running server. It is a variable so that a test can see
// that serve tears the old server down before doing anything else, which is
// the ordering that makes revoking the last device actually revoke: the
// allowlist is only read at Start, so a server left running keeps admitting
// every device that was in the file when it started.
var closeServer = (*tailcat.Server).Close

// tailcatLogf is what tailcat logs through.
//
// Its default is log.Printf, which prints everything the data plane has to say
// — netmaps, per-packet netstack lines, DNS reconfiguration — into whatever
// stderr the daemon or the app has. Dropping the "[v1]"/"[v2]" lines, which is
// how tailscale itself marks its verbose output, leaves the lines that explain
// a failure and removes the flood. Nothing here reaches a screen; a screen
// gets the word the Rust side makes out of the errno.
func tailcatLogf(format string, args ...any) {
	if strings.HasPrefix(format, "[v") {
		return
	}
	log.Printf("tailcat: "+format, args...)
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
		syscall.Close(fds[0])
		syscall.Close(fds[1])
		return -1, err
	}
	var once sync.Once
	shut := func() { once.Do(func() { ours.Close(); conn.Close() }) }
	go func() { io.Copy(ours, conn); shut() }()
	go func() { io.Copy(conn, ours); shut() }()
	return fds[1], nil
}

// fdConn adopts a raw descriptor as a net.Conn. os.NewFile takes ownership, so
// the descriptor must not be closed separately.
func fdConn(fd int) (net.Conn, error) {
	if fd < 0 {
		return nil, errors.New("not a file descriptor")
	}
	file := os.NewFile(uintptr(fd), "tailcat")
	if file == nil {
		return nil, errors.New("not a file descriptor")
	}
	defer file.Close() // FileConn dups; this closes our copy, not the peer's.
	return net.FileConn(file)
}

// clientConn is a tunneled connection that owns the client it was dialed on.
//
// One dial builds one WireGuard engine, and closing the descriptor on the Rust
// side has to take that engine with it. Tying the two together here means the
// pump's teardown path is the only teardown path there is.
type clientConn struct {
	net.Conn
	client *tailcat.Client
}

func (c *clientConn) Close() error {
	err := c.Conn.Close()
	ctx, cancel := context.WithTimeout(context.Background(), drainTimeout)
	defer cancel()
	c.client.DrainTCP(ctx)
	c.client.Close()
	return err
}

// parseNodePublic decodes a node key as the fence writes it.
func parseNodePublic(s string) (key.NodePublic, error) {
	raw, err := decodeNodeKey(s)
	if err != nil {
		return key.NodePublic{}, err
	}
	return key.NodePublicFromRaw32(mem.B(raw)), nil
}

// parseNodePrivate decodes a device's own node key.
//
// The raw 32 bytes are taken as they are, deliberately and after checking.
// Tailscale clamps when it generates a key (key.NewNode) but not when it
// parses raw bytes, so a key generated outside Go arrives here unclamped and
// it would be reasonable to expect the public key derived from it to disagree
// with the one its owner published. It does not: x/crypto's curve25519 clamps
// inside ScalarMult, so both sides land on the same public key whether or not
// this normalizes first. A clamp here was written, tested, and deleted when
// the test would not go red without it.
//
// Tailscale's own "privkey:<hex>" text form is accepted too, since a device
// that already speaks the library's language should not have to translate.
func parseNodePrivate(s string) (key.NodePrivate, error) {
	if strings.HasPrefix(s, "privkey:") {
		var priv key.NodePrivate
		if err := priv.UnmarshalText([]byte(s)); err != nil {
			return key.NodePrivate{}, err
		}
		return priv, nil
	}
	raw, err := decodeNodeKey(s)
	if err != nil {
		return key.NodePrivate{}, err
	}
	return key.NodePrivateFromRaw32(mem.B(raw)), nil
}

// decodeNodeKey turns the 43-character field into the 32 bytes it encodes.
//
// The encoding is not canonical and this does not make it so: 43 base64
// characters carry 258 bits, so the last character's low four bits are padding
// and four spellings of one key decode alike. That costs nothing here, because
// admission is decided on the decoded key rather than on the text, and
// refusing a stray padding bit would cost a runner its whole tunnel over a
// hand-edited line.
func decodeNodeKey(s string) ([]byte, error) {
	if len(s) != nodeKeyLen {
		return nil, fmt.Errorf("node key is %d characters, want %d", len(s), nodeKeyLen)
	}
	raw, err := base64.RawURLEncoding.DecodeString(s)
	if err != nil {
		return nil, err
	}
	if len(raw) != 32 {
		return nil, fmt.Errorf("node key decoded to %d bytes, want 32", len(raw))
	}
	return raw, nil
}

// parseAllowed turns the newline-separated node keys the daemon derived from
// authorized_keys into tailcat's form.
//
// A key it cannot read is an error rather than a line to skip. Skipping would
// quietly drop a device from the allowlist, and the fence has already refused
// anything but 43 base64-URL characters on both write and read-back, so a key
// arriving here that this cannot parse means something upstream is already
// wrong.
func parseAllowed(allow string) ([]key.NodePublic, error) {
	var keys []key.NodePublic
	for _, line := range strings.Split(allow, "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		k, err := parseNodePublic(line)
		if err != nil {
			return nil, err
		}
		keys = append(keys, k)
	}
	return keys, nil
}

// allowedClients is the list handed to tailcat, and it is never empty.
//
// Tailcat's AllowedClients documents "if non-empty, restricts which client
// node keys may connect" — an empty one admits everyone, because
// locoBackend.onMeow only consults the set when it is non-nil. So the zero
// key.NodePublic goes in first: nobody can hold it, since a clamped X25519
// scalar never multiplies to zero, and its presence is what makes the set a
// set rather than an absence. The tailcat CLI does the same thing for its
// "--allow none".
//
// serve refuses an empty allowlist outright, which is the guard that matters.
// This is the second one, sitting at the only place a tailcat.Server is built,
// so that deleting the first cannot turn a runner into an open one.
func allowedClients(keys []key.NodePublic) []key.NodePublic {
	return append([]key.NodePublic{{}}, keys...)
}

// loadOrCreateKey returns this runner's persistent node identity.
//
// Its existence is the feature flag: no file, no server, no DERP connection.
// It is created mode 0600 and refused if it is group- or world-readable, the
// same posture the fence takes about authorized_keys — a node key that anyone
// on the runner can read is a route anyone on the runner can take.
func loadOrCreateKey(path string) (key.NodePrivate, error) {
	file, err := os.OpenFile(path, os.O_RDONLY, 0)
	if errors.Is(err, os.ErrNotExist) {
		return createKey(path)
	}
	if err != nil {
		return key.NodePrivate{}, err
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil {
		return key.NodePrivate{}, err
	}
	if !info.Mode().IsRegular() {
		return key.NodePrivate{}, fmt.Errorf("%s is not a regular file", path)
	}
	if perm := info.Mode().Perm(); perm&0o077 != 0 {
		return key.NodePrivate{}, fmt.Errorf("%s is mode %04o, want 0600", path, perm)
	}
	data, err := io.ReadAll(io.LimitReader(file, 4096))
	if err != nil {
		return key.NodePrivate{}, err
	}
	var priv key.NodePrivate
	if err := priv.UnmarshalText([]byte(strings.TrimSpace(string(data)))); err != nil {
		return key.NodePrivate{}, err
	}
	if priv.IsZero() {
		return key.NodePrivate{}, fmt.Errorf("%s holds no key", path)
	}
	return priv, nil
}

// createKey writes a fresh identity, and fails rather than overwriting one
// that appeared between the read above and this call.
func createKey(path string) (key.NodePrivate, error) {
	priv := key.NewNode()
	text, err := priv.MarshalText()
	if err != nil {
		return key.NodePrivate{}, err
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return key.NodePrivate{}, err
	}
	if _, err := file.Write(append(text, '\n')); err != nil {
		file.Close()
		os.Remove(path)
		return key.NodePrivate{}, err
	}
	if err := file.Close(); err != nil {
		os.Remove(path)
		return key.NodePrivate{}, err
	}
	return priv, nil
}

// stopServer closes whatever server is running and forgets it.
//
// There is no way to withdraw one key from a live server: the allowlist is
// copied into the backend at Start and consulted only when a client first
// registers, so a client already peered never passes that check again.
// Rebuilding the server is not merely the available way to revoke a route, it
// is the only one that works — and it drops every live tunnel, not only the
// revoked device's.
func stopServer() {
	if server == nil {
		return
	}
	closeServer(server)
	server = nil
}

// serve replaces this runner's tunnel server.
//
// Every call tears down whatever was running first, so the running server
// always reflects the last allowlist this was given and a stale one cannot
// outlive a revocation. An allowlist that cannot be used leaves the runner
// with no server at all, which is the correct end state for a runner that
// admits nobody: tailcat reads an empty AllowedClients as "admit everyone", so
// forwarding one would open this runner's sshd to anyone holding the token.
func serve(keyPath string, sshPort uint16, allow string) int {
	mu.Lock()
	defer mu.Unlock()
	stopServer()

	allowed, err := parseAllowed(allow)
	if err != nil || len(allowed) == 0 {
		return -int(syscall.EINVAL)
	}
	priv, err := loadOrCreateKey(keyPath)
	if err != nil {
		return -int(syscall.EACCES)
	}
	s := &tailcat.Server{
		Key:            priv,
		Logf:           tailcatLogf,
		AllowedClients: allowedClients(allowed),
		DERPMapURL:     derpMapURL,
		// The packet filter drops anything but the one port this runner
		// serves before it reaches netstack. OnTCP below is still the gate;
		// this is the cheaper one in front of it.
		ServedTCPPorts: []filter.PortRange{{First: tunnelPort, Last: tunnelPort}},
		OnTCP: func(port uint16) func(net.Conn) {
			if port != tunnelPort {
				return nil // no handler is a RST
			}
			return func(c net.Conn) {
				// The port the client asked for is a name, not an address.
				// Where sshd actually listens is a fact only this runner has.
				local, err := net.Dial("tcp", net.JoinHostPort("127.0.0.1", strconv.Itoa(int(sshPort))))
				if err != nil {
					c.Close()
					return
				}
				go func() { io.Copy(local, c); local.Close() }()
				io.Copy(c, local)
				c.Close()
			}
		},
	}
	if err := s.Start(); err != nil {
		return -int(syscall.EIO)
	}
	server = s
	return 0
}

// allowAdd admits one more device to the running server.
//
// This is the migration path — a device that already holds an authenticated
// SSH session registers its node key over it — and it deliberately does not
// rebuild: adding a route for one device must not drop every other device's
// live tunnel. Withdrawing one still does, because tailcat offers nothing
// finer; see stopServer.
func allowAdd(nodeKey string) int {
	k, err := parseNodePublic(nodeKey)
	if err != nil {
		return -int(syscall.EINVAL)
	}
	if k.IsZero() {
		// The seed key, which nobody can hold. Admitting it is a no-op that
		// would read as success.
		return -int(syscall.EINVAL)
	}
	mu.Lock()
	defer mu.Unlock()
	if server == nil {
		return -int(syscall.ENOTCONN)
	}
	server.AddAllowedClient(k)
	return 0
}

// connBlob copies this runner's token into buf, NUL-terminated, and returns
// the length of the token itself.
func connBlob(buf []byte) int {
	mu.Lock()
	defer mu.Unlock()
	if server == nil {
		return -int(syscall.ENOTCONN)
	}
	blob := string(server.ConnBlob())
	if len(buf) < len(blob)+1 {
		return -int(syscall.ERANGE)
	}
	copy(buf, blob)
	buf[len(blob)] = 0
	return len(blob)
}

// dial opens a tunneled connection to a runner and hands back a descriptor.
func dial(token, clientKey string, port uint16) int {
	if token == "" {
		return -int(syscall.EINVAL)
	}
	priv, err := parseNodePrivate(clientKey)
	if err != nil {
		return -int(syscall.EINVAL)
	}
	mu.Lock()
	mapURL := derpMapURL
	mu.Unlock()
	client := &tailcat.Client{
		Server:     tailcat.ConnBlob(token),
		Key:        priv,
		Logf:       tailcatLogf,
		DERPMapURL: mapURL,
	}
	conn, err := client.DialTCPPort(context.Background(), port)
	if err != nil {
		client.Close()
		return -int(syscall.EHOSTUNREACH)
	}
	fd, err := pumpToSocketpair(&clientConn{Conn: conn, client: client})
	if err != nil {
		conn.Close()
		client.Close()
		return -int(syscall.EMFILE)
	}
	return fd
}

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
	mu.Lock()
	defer mu.Unlock()
	derpMapURL = C.GoString(url)
	return 0
}

func main() {}
