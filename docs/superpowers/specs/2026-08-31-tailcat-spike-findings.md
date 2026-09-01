# What the tailcat spike measured, and what it decided

Phase 0 of `2026-08-31-tailcat-transport-design.md`. Six unknowns, each with a
branch chosen in advance so the spike shapes the work instead of gating it.
Everything below was measured on 2026-08-31 against `github.com/tailscale/tailcat`
**v0.4.0** (which pins `tailscale.com v1.103.0-pre.0.20260830144538-72780705eda8`),
Go 1.27.0 darwin/arm64, Xcode at `/Applications/Xcode.app`, on this Mac.

The spike code was throwaway and lives in no repository. The iOS project was
patched, measured, and restored byte-for-byte; `git status` shows no iOS change.

## The number that goes first

**Linking tailcat's client into the iOS app grew the app by 20,349,912 bytes —
19.41 MiB — of `arm64` machine code, and 7,224,430 bytes (6.89 MiB) compressed.**

That is a measurement, not the spec's 15–25 MB guess. Details and method in
finding 3.

## What could not be measured, stated up front

The owner's iPhone reports `unavailable`. Three things therefore remain open,
and none of them is inferred below:

- **`fc_probe()` returning 42 on a real device.** The link is proved; the
  runtime is not. See finding 2.
- **Tunnel survival across app suspension.** Entirely unmeasured. Finding 4.
- **Time to first byte on cellular.** LAN measured, cellular unmeasured.
  Finding 5.

---

## 1. Allowlist removal

**There is no removal method. Verbatim, the complete `Server` method set at
v0.4.0:**

```
func (s *Server) AddAllowedClient(k key.NodePublic)
func (s *Server) Addr() netip.Addr
func (s *Server) Close() error
func (s *Server) ConnBlob() ConnBlob
func (s *Server) DrainTCP(ctx context.Context) error
func (s *Server) HandleTailscaleSSHConn(c net.Conn)
func (s *Server) SSHConnHandler(opts SSHOptions) func(net.Conn)
func (s *Server) Start() error
func (s *Server) Status() *ipnstate.Status
```

`grep -iE 'allow|remove|revoke'` over the whole package doc returns exactly two
allowlist items — the `AllowedClients` field and `AddAllowedClient` — plus the
unrelated `AllowProxy` hook. There is no `RemoveAllowedClient`, no
`SetAllowedClients`, and no exported accessor that would let a caller reach the
live set.

The field's own documentation says why:

```go
// AllowedClients, if non-empty, restricts which client node keys
// may connect; all others are silently ignored. If empty, all
// clients are allowed. See [Server.AddAllowedClient] to add more
// at runtime.
AllowedClients []key.NodePublic
```

Three details from the source that matter more than the absence itself
(`tailcat.go`, v0.4.0):

1. **The exported field is read once, at `Start`.** Line 429 copies it into the
   backend's unexported `allowedClients` map. Mutating `s.AllowedClients` after
   `Start` changes nothing. `AddAllowedClient` (line 680) knows this and
   branches on whether `s.lb` is nil.
2. **The allowlist is a registration gate, not a traffic gate.** It is consulted
   in exactly one place — `locoBackend.onMeow`, line 1366 — which is where a
   client first announces itself and gets added as a WireGuard peer. A client
   already in `b.clients` never passes that check again. So even a hypothetical
   `RemoveAllowedClient` would not close a tunnel already open; it would only
   stop the next registration. **Rebuilding the `Server` is not merely the
   available option, it is the only one that actually withdraws a live route.**
3. **An empty allowlist means everyone.** Revoking the last enrolled device by
   rebuilding a `Server` with an empty `AllowedClients` would turn a restricted
   runner into one that accepts any client holding the token — the exact
   opposite of revoking. The tailcat CLI solves this by inserting a key nobody
   can hold (`cmd/tailcat/tailcat.go:1153`):

   ```go
   if ks == "none" {
       s.AddAllowedClient(key.NodePublic{})
   }
   ```

   **Task 3 must copy this.** A rebuild after the last revocation seeds the
   allowlist with the zero `key.NodePublic`, never with nothing.

**Branch selected: "If it does not."**

Downstream:

- **Task 3** implements no `allow_remove`. It implements a rebuild instead, and
  it must seed the zero key when the resulting allowlist would be empty.
- **Task 8**'s `revoke` rebuilds the `Server` rather than calling a removal.
- **Task 12** keeps the `runner.restart()` line.
- **`docs/runners.md` must say that revocation drops live tunnels** — every
  enrolled device's tunnel, not only the revoked one, because the rebuild
  replaces the server all of them are peered with. This is defensible for an
  operation that already closes every session it revokes, but it is a fact the
  documentation owes the reader rather than one they discover.

## 2. iOS linking

**Holds for linking. Runtime unverified — no device was available.**

The c-archive built on the first attempt with the brief's command exactly as
written, no added flags:

```bash
CGO_ENABLED=1 GOOS=ios GOARCH=arm64 \
  CC="$(xcrun --sdk iphoneos -f clang) -isysroot $(xcrun --sdk iphoneos --show-sdk-path) -arch arm64" \
  go build -buildmode=c-archive -o libprobe.a probe.go
```

`lipo -info libprobe.a` → `Non-fat file: libprobe.a is architecture: arm64`.
25 s cold, 43,088,280 bytes of archive for the minimal probe.

Linking it into the app took two changes to `apps/ios/generate-project.py`, both
of which Task 5 will need:

1. The archive is packaged as an XCFramework the same way the Rust staticlibs
   are (`xcodebuild -create-xcframework -library libprobe.a -headers <stage>`),
   with a hand-written `module.modulemap` beside the cgo-generated header, in a
   subdirectory named for the module — the same collision-avoidance the
   `build-ios-frameworks.sh` comment already explains. Added to `FRAMEWORKS`.
2. **`SWIFT_INCLUDE_PATHS` must be extended too.** `HEADER_SEARCH_PATHS` uses
   `$(BUILT_PRODUCTS_DIR)/include/**`, but Swift resolves the Clang module by
   `SWIFT_INCLUDE_PATHS`, which names each module directory explicitly. Without
   the new entry the build fails with `unable to resolve module dependency:
   'TailcatProbe'` and nothing points at the search path. This is the one trap
   in an otherwise uneventful integration.

With those, from a Swift call site in `FarCoolerApp.body`:

```
xcodebuild -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
  -configuration Release -destination "generic/platform=iOS" \
  CODE_SIGNING_ALLOWED=NO build
→ ** BUILD SUCCEEDED **
```

No duplicate symbols against `libfarcooler_client.a` (which carries BoringSSL,
so a clash was plausible and did not happen). `nm` on the resulting device
binary shows `T _fc_probe`, `T _fc_dial`, 1,965 `runtime.*` symbols, and 4,723
symbols matching `gvisor|netstack|wireguard|magicsock` — the whole data plane is
in the binary, and `strings` finds `go1.27.0` in it.

**What is proved:** a Go `c-archive` compiles for `ios-arm64` and links into
this app beside the Rust staticlib, device-targeted, with no linker
intervention.

**What is not proved:** that `fc_probe()` returns 42 when the app runs. That
requires hardware. It was not run on the Simulator and called equivalent — a
Simulator build is a different platform triple and, per the standing note on
this repository, lies about platform limits. **Whoever first has a device
should run the app once and check that `fc_probe()` returns 42 before Task 5's
iOS half is considered done.**

**Branch selected: "If it holds."** The failure branch exists for a link that
does not work; this one does.

Downstream: Task 5 proceeds in full, including step 3. Task 10's iOS half
proceeds. The generator change in point 2 above is a required part of Task 5,
not an optional nicety.

## 3. Size

Method: `-configuration Release`, `-destination "generic/platform=iOS"`,
`CODE_SIGNING_ALLOWED=NO`, into a scratch `-derivedDataPath`. The same Rust
xcframeworks in both builds. Sizes are the sum of every regular file in
`FarCooler.app`; the compressed column is `ditto -c -k --keepParent` of the
`.app`, which is what an IPA's `Payload/` is a zip of, and is therefore the
closer proxy for a download.

Two "after" builds, because the brief's probe understates the case: it only
declares `var s tailcat.Server`, and Go's linker dead-strips. The second probe
calls what a phone actually calls — `ParseConnBlob`, `NewClient`, `Ping`,
`DiscoPing`, `DialTCPPort`, `Close` — and is the honest number.

| Build | Main binary | All files in `.app` | Compressed |
|---|---|---|---|
| Before | 18,595,032 | 52,659,989 | 18,765,556 |
| After, minimal probe (`var s tailcat.Server`) | 37,027,152 | 71,092,109 | 25,323,667 |
| **After, real client surface** | **38,944,944** | **73,009,901** | **25,989,986** |

**Delta, real client surface: +20,349,912 bytes = 19.41 MiB of binary;
+7,224,430 bytes = 6.89 MiB compressed.** Every added byte is in the main
executable; no other file in the bundle changed.

Caveats, so the number is not repeated as more than it is:

- It is an unsigned `.app`, not an App Store IPA. Apple's own recompression and
  encryption move this, usually upward on disk and downward on download.
- The Rust staticlibs were the ones already in `apps/ios/Frameworks/` and were
  not rebuilt. That is fine for a delta — both builds link the same ones — and
  irrelevant to what the delta measures.
- Only the **client** side of tailcat is exercised. A runner also calls
  `Server.Start`, `OnTCP` and the netstack listen path; the daemon's Go archive
  will be larger than 19.41 MiB. That is a Linux and macOS binary, where the
  number matters less.
- The minimal-probe row is 1.9 MiB smaller, which is the measure of how much
  dead-stripping is left on the table: not much. There is no configuration of
  this dependency that is meaningfully cheaper.

**Branch selected: "Proceed."** 19.41 MiB is inside the spec's guessed 15–25 MB
band, so nothing is surprising, but the guess is now retired: the number is
19.41 MiB uncompressed and 6.89 MiB on the wire.

Downstream: Task 5 proceeds unchanged. Whether a phone app may grow by 6.89 MiB
of download is a product decision, not an engineering one, and the code is
identical either way — do not gate, defer, or quietly drop the iOS half over
this.

## 4. Suspension

**Unmeasured. No device was available, and this cannot be faked.**

Nothing below is a measurement; it is the state of the code the branch says to
rely on.

The decision table's branch is "proceed" whichever way this falls, and the
mechanism it names is already in the tree. `crates/client/src/ssh.rs:143-149`:

```rust
let config = Arc::new(Config {
    // A phone sleeps and wakes; without keepalives a session that the
    // network dropped hours ago looks alive until the first write.
    keepalive_interval: Some(std::time::Duration::from_secs(30)),
    keepalive_max: 3,
    ..Config::default()
});
```

A tunnel that does not survive suspension surfaces as a dead SSH session, and
that configuration notices within roughly 90 seconds (three missed keepalives at
30-second intervals) rather than at the first write. **Add no new reconnect
machinery in any task.**

The cost that is therefore unknown: how long a wake-and-reconnect takes end to
end. Finding 5 puts the tunnel's own share of that at roughly 170 ms on this
network, so the reconnect is dominated by SSH authentication and by however long
the keepalive takes to declare the old session dead — not by tailcat.

**Still to do when a device exists:** open a tunnel, background the app 60 s,
foreground it, write, and record whether the tunnel survived and what the
reconnect cost. Until then no claim about pocket behavior is supported.

## 5. Time to first byte

**Measured on LAN. Not measured on cellular.**

Measurements are from this Mac on Wi-Fi, with a tailcat server and a tailcat
client as two separate processes, so every number includes full process startup
— which is what a per-`git fetch` `ProxyCommand` actually pays.

Against Tailscale's public DERP (region 302, `sfo`; netcheck reported 6–7 ms to
the relay):

- `tailcat ping`: **15.2–19.2 ms** per pong via DERP, over ten pings. That is
  the relay round trip, twice the distance to the relay.
- **Cold process start to the first byte of a response through the tunnel:
  113, 125, 172, 164, 171, 175 ms** — six runs, median ~168 ms. Measured by
  `Popen`, writing an HTTP request to stdin, and stopping the clock on
  `stdout.read(1)`.

Against a local `derper` on loopback, which removes the network from the
handshake entirely and so gives the floor:

- `tailcat ping`: **310–410 µs**.
- Cold start to first byte: **70, 75, 75, 84 ms** — four runs.

So the cost decomposes as roughly **75 ms of fixed startup** (Go runtime,
netcheck, WireGuard engine bring-up) **plus about six to seven round trips to
the DERP relay**: TCP and TLS to the relay, the meow/meowed registration that
travels client → relay → server → relay → client, and the WireGuard handshake.
At 7 ms to the relay that is ~170 ms total. The verbose trace confirms the
shape: `magicsock: new contact: peer=... usec=44952 ... via=derp`, then
`Sending handshake initiation` / `Received handshake response` in the same
second.

Two things the measurement is not:

- **Cellular is unmeasured.** The arithmetic above says the fixed 75 ms stays
  and the round-trip term scales; a relay 60 ms away would land near 500 ms, and
  one 150 ms away near 1.1 s. That is extrapolation, not measurement, and is
  written here as such.
- **Direct-path upgrade is unreliable and slow, even here.** `tailcat ping
  --until-direct --timeout=20s`, three runs between two processes on this same
  machine: one upgraded at **11.0 s** (to an IPv6 endpoint), two never upgraded
  inside 20 s. The DERP path carries traffic the whole time, so this costs
  correctness nothing — but no design should assume a tunnel becomes direct
  promptly, or at all.

**Branch selected: "If it holds" — do not add `ControlMaster` in Task 11.**

The table's condition is a measurement showing the handshake is cheap, and there
is one: 168 ms median, cold, including process startup. A control socket buys
back a sixth of a second per `git fetch` and pays for it with stale state
pointing at a tunnel that can die underneath it — and finding 4 says a tunnel
dying underneath it is the expected case on a phone, not the exotic one.

Downstream, and the condition under which this flips: Task 11 step 5's
`ControlMaster` block is omitted. **If a cellular measurement ever puts cold
time to first byte above about one second, reopen this** — the block is already
written out in that task and adding it is a small change.

## 6. Test harness

**Holds. `derper -dev` serves a local harness with no TLS, and two tailcat peers
reach each other through it.** One detail the brief did not have is required,
and without it the harness silently fails; it is written out below.

`go install tailscale.com/cmd/derper@latest` built cleanly (13 s). Then:

```
$ derper -dev -a :3340
Running in dev mode.
STUN server listening on [::]:3478
derper: serving on :3340
```

`-dev` overrides `-a` and happens to choose the same `:3340`. Plain HTTP:
`curl -s -i localhost:3340/derp/probe` → `HTTP/1.1 200 OK`, and
`/generate_204` → `204`. A self-signed cert with `-hostname localhost` was never
needed.

**The DERP map that points two peers at it**, verified end to end:

```json
{
  "Regions": {
    "900": {
      "RegionID": 900,
      "RegionCode": "local",
      "RegionName": "Local derper",
      "Nodes": [
        {
          "Name": "900a",
          "RegionID": 900,
          "HostName": "localhost",
          "IPv4": "127.0.0.1",
          "IPv6": "::1",
          "STUNPort": 3478,
          "DERPPort": 3340,
          "InsecureForTests": true
        }
      ]
    }
  }
}
```

**`InsecureForTests` is not enough, and this is the trap.** In
`tailscale.com/derp/derphttp`, `InsecureForTests` only sets
`InsecureSkipVerify` on a TLS handshake that still happens; the client keeps
speaking TLS to a plain-HTTP derper and fails with `tls: first record does not
look like a TLS handshake`, retrying behind a backoff until the caller's
deadline. The only switch that selects plain HTTP for a region-addressed DERP
node is the environment knob `debugUseDERPHTTP`
(`derphttp_client.go:251,264`):

```
TS_DEBUG_USE_DERP_HTTP=1
```

It must be set **in both peer processes**, before the first DERP connection.
With it, `tailcat ping` over the local derper returns `pong in 310µs via
DERP(local)`; without it, `ping: context deadline exceeded`. It is registered
through `envknob`, so a test must set it before the process reads it, not
mid-run.

Two more harness details worth having:

- **Give the server an embedded region, or tell the client the map URL.** A
  server's default token carries only `RegionID: 900`, and a client with no
  `--derpmap-url` then fails with `connection string said only DERP RegionID
  900 but no such region in https://tailcat.dev/derpmap.json` — a test would go
  to the public internet to fail. The CLI's `--full-address` (the library's
  `Server.ConnBlob` with `Server.Region` populated) embeds the whole node,
  ports and `InsecureForTests` included, and then the client needs nothing but
  the env knob. **This is also the right choice for the product's QR token:** it
  removes a fetch of `tailcat.dev/derpmap.json` from a phone's first connect.
- The server must also be told the map. `--derpmap-url http://127.0.0.1:PORT/derpmap.json`
  works; it logged `Selected bootstrap relay region 900, Local derper`.

End to end over the local derper, both peers on this Mac: an HTTP request
written into `tailcat <blob> <port>` came back with `HTTP/1.0 200 OK`, first
byte at 70–84 ms.

**Branch selected: "If it holds."**

Downstream: **Task 13** (and Task 12's end-to-end test) is written against the
local derper. Nothing is marked `#[ignore]`. The harness must set
`TS_DEBUG_USE_DERP_HTTP=1` and use a full-address token; a test that skips
either will hang until its timeout while quietly trying to reach the public
internet, which is precisely the failure this repository is named for.

---

## Nothing here changed the plan's shape

All six branches landed on "proceed". The plan is executable as written, with
four concrete amendments:

1. **Task 3** rebuilds the `Server` to revoke, and seeds `key.NodePublic{}` when
   the allowlist would otherwise be empty. No `allow_remove`.
2. **Task 5** must extend `SWIFT_INCLUDE_PATHS`, not only `FRAMEWORKS`, in
   `apps/ios/generate-project.py`.
3. **Task 11** omits the `ControlMaster` block.
4. **Task 13** sets `TS_DEBUG_USE_DERP_HTTP=1` and uses a full-address token.

And three things stay open until hardware exists: `fc_probe()` at runtime,
suspension survival, and cellular time to first byte.
