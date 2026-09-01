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
	"tailscale.com/tailcfg"
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

// dialTimeout bounds the TCP connect through an established tunnel. The
// handshake before it bounds itself: Client.Ping times out after ten seconds
// whatever context it is given.
const dialTimeout = 30 * time.Second

// regionPickTimeout bounds the one netcheck a runner runs on its first start,
// to choose the relay it then keeps.
const regionPickTimeout = 15 * time.Second

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

// recoverToErrno stops a panic in the data plane from taking the whole process
// with it.
//
// Two of them are reachable from here and neither is hypothetical. The
// server's connBlob panics on "no DERPMap set" and "no regions in derpmap",
// synchronously under fc_tailcat_conn_blob. Client.DrainTCP reaches
// tcpipStackOf, which is reflection into netstack's unexported ipstack field
// and which upstream says panics on purpose if the tailscale dependency
// drifts — and that one runs in a goroutine this package owns. An unrecovered
// panic in either aborts farcoolerd, or the app on a phone.
//
// This only works because these are our goroutines: recover cannot reach a
// panic raised on a stack we did not start.
func recoverToErrno(rc *int) {
	if r := recover(); r != nil {
		log.Printf("tailcat: recovered from a panic in the data plane: %v", r)
		*rc = -int(syscall.EIO)
	}
}

// recoverInGoroutine is the same barrier where there is no caller to answer:
// the copy goroutines and the teardown they run.
func recoverInGoroutine() {
	if r := recover(); r != nil {
		log.Printf("tailcat: recovered from a panic while tearing a tunnel down: %v", r)
	}
}

// tailcatLogf is what tailcat logs through.
//
// Its default is log.Printf, which prints everything the data plane has to say
// — netmaps, per-packet netstack lines, DNS reconfiguration — into whatever
// stderr the daemon or the app has. Dropping the "[v1]"/"[v2]" lines, which is
// how tailscale itself marks its verbose output, leaves the lines that explain
// a failure and removes the flood. Nothing here reaches a screen; a screen
// gets the word the Rust side makes out of the errno.
//
// The marker is matched anywhere in the format, not only at the front: a live
// run showed most of the volume arriving already prefixed by its subsystem, as
// "wg: [v2] …" and "netcheck: [v1] …", which a HasPrefix test lets straight
// through.
func tailcatLogf(format string, args ...any) {
	if strings.Contains(format, "[v1] ") || strings.Contains(format, "[v2] ") {
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
	go func() { defer recoverInGoroutine(); io.Copy(ours, conn); shut() }()
	go func() { defer recoverInGoroutine(); io.Copy(conn, ours); shut() }()
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
// characters carry 258 bits, so the last character's low TWO bits are padding
// and four spellings of one key ("…AAA" through "…AAD") decode alike. Refusing
// a stray padding bit would cost a runner its whole tunnel over a hand-edited
// line, and admission is decided on the decoded key rather than on the text.
//
// The consequence belongs on the record rather than in a report: two
// authorized_keys lines spelling one key differently are two devices to
// `fence` and one key to the runner, so revoking one leaves the route open
// through the other. It grants nothing — a route, never a permission, and the
// surviving line still has to authenticate over SSH — but it argues for
// canonicalizing the field in `fence`, where the file is written.
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

// identity is what a runner is on the tunnel: a persistent private key, and
// the DERP region its token names.
//
// The region is pinned because the token is not a lookup. Upstream picks the
// nearest region by latency at Start whenever RegionID is zero, so a daemon
// restarted on a different network picks a different relay and every token
// already sitting in a device's manifest names a region this runner no longer
// listens on. Nothing tells the device that; it just stops connecting.
type identity struct {
	priv     key.NodePrivate
	regionID tailcfg.DERPRegionID
}

// The file is two lines, the second written only once a region has been
// chosen:
//
//	privkey:<64 hex>
//	region <id>
const identityRegionPrefix = "region "

// loadOrCreateIdentity returns this runner's persistent tunnel identity,
// creating the file on first use.
//
// Its existence is the feature flag: no file, no server, no DERP connection.
// It is created mode 0600 and refused if it is group- or world-readable, the
// same posture the fence takes about authorized_keys — a node key that anyone
// on the runner can read is a route anyone on the runner can take.
func loadOrCreateIdentity(path string) (identity, error) {
	data, err := readPrivateFile(path)
	if errors.Is(err, os.ErrNotExist) {
		return createIdentity(path)
	}
	if err != nil {
		return identity{}, err
	}
	var id identity
	for _, line := range strings.Split(string(data), "\n") {
		line = strings.TrimSpace(line)
		switch {
		case line == "":
		case strings.HasPrefix(line, identityRegionPrefix):
			n, err := strconv.Atoi(strings.TrimSpace(strings.TrimPrefix(line, identityRegionPrefix)))
			if err != nil {
				return identity{}, fmt.Errorf("%s: unreadable region: %w", path, err)
			}
			id.regionID = tailcfg.DERPRegionID(n)
		case id.priv.IsZero():
			if err := id.priv.UnmarshalText([]byte(line)); err != nil {
				return identity{}, err
			}
		default:
			return identity{}, fmt.Errorf("%s: unexpected line", path)
		}
	}
	if id.priv.IsZero() {
		return identity{}, fmt.Errorf("%s holds no key", path)
	}
	return id, nil
}

// readPrivateFile reads a file this runner keeps to itself, refusing one
// anybody else can read.
//
// The mode is checked on the open handle rather than on the path, so there is
// no window between deciding a file is private and reading it.
func readPrivateFile(path string) ([]byte, error) {
	file, err := os.OpenFile(path, os.O_RDONLY, 0)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	info, err := file.Stat()
	if err != nil {
		return nil, err
	}
	if !info.Mode().IsRegular() {
		return nil, fmt.Errorf("%s is not a regular file", path)
	}
	if perm := info.Mode().Perm(); perm&0o077 != 0 {
		return nil, fmt.Errorf("%s is mode %04o, want 0600", path, perm)
	}
	return io.ReadAll(io.LimitReader(file, 4096))
}

// createIdentity writes a fresh key, and fails rather than overwriting one
// that appeared between the read above and this call. It carries no region:
// which relay this runner listens on is not known until Start has picked one.
func createIdentity(path string) (identity, error) {
	id := identity{priv: key.NewNode()}
	text, err := renderIdentity(id)
	if err != nil {
		return identity{}, err
	}
	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return identity{}, err
	}
	if _, err := file.Write(text); err != nil {
		file.Close()
		os.Remove(path)
		return identity{}, err
	}
	if err := file.Close(); err != nil {
		os.Remove(path)
		return identity{}, err
	}
	return id, nil
}

func renderIdentity(id identity) ([]byte, error) {
	text, err := id.priv.MarshalText()
	if err != nil {
		return nil, err
	}
	text = append(text, '\n')
	if id.regionID != 0 {
		text = append(text, fmt.Sprintf("%s%d\n", identityRegionPrefix, id.regionID)...)
	}
	return text, nil
}

// pinRegion records the region Start chose, so the next start names the same
// relay and the tokens already in devices' manifests keep resolving.
//
// It writes a new file and renames it over the old one, because the key is in
// there too: a truncating rewrite interrupted halfway leaves a runner with no
// identity at all, which is a lost fleet rather than a lost pin.
func pinRegion(path string, id identity) error {
	text, err := renderIdentity(id)
	if err != nil {
		return err
	}
	tmp := path + ".new"
	os.Remove(tmp)
	file, err := os.OpenFile(tmp, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		return err
	}
	if _, err := file.Write(text); err != nil {
		file.Close()
		os.Remove(tmp)
		return err
	}
	if err := file.Close(); err != nil {
		os.Remove(tmp)
		return err
	}
	if err := os.Rename(tmp, path); err != nil {
		os.Remove(tmp)
		return err
	}
	return nil
}

// chooseRegion picks the relay this runner will listen on, so that the choice
// can be written down.
//
// Start makes the same choice when RegionID is zero, but it makes it inside
// itself and hands back no way to ask what it chose: the region is not on the
// Server, and the token is no help either — ConnInfo.ConnBlob zeroes the
// region ID before encoding and ParseConnBlob restores it as an index, so a
// token minted in region 302 parses back as region 1. Measured, not assumed.
// Doing the pick here costs one netcheck on a runner's very first start and
// nothing on every start after it.
func chooseRegion(mapURL string) (tailcfg.DERPRegionID, error) {
	ctx, cancel := context.WithTimeout(context.Background(), regionPickTimeout)
	defer cancel()
	opts := []any{tailcat.ExpandForServer}
	if mapURL != "" {
		opts = append(opts, tailcat.DERPMapURL(mapURL))
	}
	dm, err := tailcat.FetchDERPMap(ctx, opts...)
	if err != nil {
		return 0, err
	}
	return tailcat.PickBestRegion(ctx, dm)
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
func serve(keyPath string, sshPort uint16, allow string) (rc int) {
	defer recoverToErrno(&rc)
	mu.Lock()
	defer mu.Unlock()
	stopServer()

	allowed, err := parseAllowed(allow)
	if err != nil || len(allowed) == 0 {
		return -int(syscall.EINVAL)
	}
	id, err := loadOrCreateIdentity(keyPath)
	if err != nil {
		return -int(syscall.EACCES)
	}
	if id.regionID == 0 {
		// The pin is a convenience, not a precondition. A runner whose region
		// could not be chosen or written down is reachable right now on the
		// token it is about to mint, and pays for it at the next restart by
		// choosing again; refusing to serve over a failed write would be the
		// worse trade.
		if picked, err := chooseRegion(derpMapURL); err != nil || picked == 0 {
			log.Printf("tailcat: serving without a pinned DERP region (%v); the token may move on restart", err)
		} else if err := pinRegion(keyPath, identity{priv: id.priv, regionID: picked}); err != nil {
			log.Printf("tailcat: could not pin DERP region %d: %v", picked, err)
		} else {
			id.regionID = picked
		}
	}
	s := &tailcat.Server{
		Key:            id.priv,
		RegionID:       id.regionID,
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
func allowAdd(nodeKey string) (rc int) {
	defer recoverToErrno(&rc)
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
func connBlob(buf []byte) (rc int) {
	defer recoverToErrno(&rc)
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
//
// The handshake is run explicitly, in two legs, because the spec asks for two
// different sentences and a single call cannot tell them apart. "Can't reach
// the rendezvous service" and "This runner didn't answer, its access may have
// been revoked" are different facts about different parties, and an
// unrecognized client is ignored SILENTLY by the runner — the only thing a
// revoked device would otherwise get is a generic timeout, which is exactly
// what this tree keeps committing fixes against.
//
// The seam is upstream's own: Client.Ping starts the client and then waits for
// the runner's acknowledgment under a ten-second timer of its own. A deadline
// means the meow went out and nothing came back — the runner. Anything else
// failed before that, resolving the relay or bringing the engine up — the
// rendezvous. (A token whose DERP map had to be fetched could in principle
// time out on the first leg too; ours never does, because Server.ConnBlob
// always embeds the region.) A failure after both legs is the tunnel working
// and sshd not answering on the other side.
func dial(token, clientKey string, port uint16) (rc int) {
	defer recoverToErrno(&rc)
	if token == "" {
		return -int(syscall.EINVAL)
	}
	priv, err := parseNodePrivate(clientKey)
	if err != nil {
		return -int(syscall.EINVAL)
	}
	if priv.IsZero() {
		// Upstream reads a zero Key as "generate an ephemeral one", so a key
		// of 43 zero characters would dial under an identity that is in no
		// allowlist, be ignored without a word, and spend the full ten
		// seconds arriving at a timeout that names the wrong thing. The same
		// refusal loadOrCreateIdentity and allowAdd already make.
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
	ctx, cancel := context.WithTimeout(context.Background(), dialTimeout)
	defer cancel()
	if _, err := client.Ping(ctx); err != nil {
		client.Close()
		if errors.Is(err, context.DeadlineExceeded) {
			return -int(syscall.ETIMEDOUT)
		}
		return -int(syscall.EHOSTUNREACH)
	}
	conn, err := client.DialTCPPort(ctx, port)
	if err != nil {
		client.Close()
		return -int(syscall.ECONNREFUSED)
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
