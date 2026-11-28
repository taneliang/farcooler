# ov-1502: a nearer path for web sessions

Research only. No product code changed, no relay flag flipped, no secret created, nothing deployed. Spike code is in `reports/ov-1502/spike/`. Measured on this Mac on Nov 28, 2026, between 20:45 and 21:20 local.

## Answer

- **Today a keystroke's echo costs 360 ms in transport alone.** The runner's relay is pinned to San Francisco. This is measured, not estimated (table below).
- **A direct WebRTC data channel is viable.** From Chromium and WebKit into a Rust runner built on webrtc-rs, SSH runs unchanged inside the channel, and the echo cost **0.4 ms**. Through a TURN relay the channel opened in **0.34 to 0.43 s**.
- **Cloudflare TURN as the fallback costs $0 a month at our volume.** Its nearest location is **6 ms** from this Mac, so a relayed echo should take about 12 to 15 ms. That figure is estimated: there was no key to test with.
- **Recommendation:** go ahead with the owner's preferred design: WebRTC direct first, Cloudflare STUN and TURN as lookup and fallback, then delete Tailcat. Two findings need handling in the plan:
  1. webrtc-rs can use TURN only over UDP on the runner side.
  2. The macOS firewall blocks incoming UDP to unsigned binaries.
- **Smallest first ticket:** "Relay: carry WebRTC offers to the runner" (details under "Cutover tickets").

## 1. Keystroke-to-echo, today and per option

### How it was measured

`spike/echo_rtt.py` writes one byte into the transport and times its return. A TCP echo server on the runner side stands in for sshd and the pty. Each row is the median of 100 to 200 keystrokes after warm-up.

The real web terminal adds three things on top:

- SSH encryption: microseconds.
- The pty's echo: under 1 ms.
- Today only, the browser's WebAssembly network stack, which I did not measure.

So today's figure is a lower bound for the browser.

### Results

| Path | Echo, median (p90) | How known | Monthly cost |
|---|---|---|---|
| **Today**: Tailcat over DERP `sfo` (region 302, the runner's pin) | **360 ms** (363) | measured, native `tailcat` with `TS_DEBUG_ALWAYS_USE_DERP=1` | $0 (Tailscale's free, rate-limited relays) |
| Same, DERP `fra` | 336 ms (342) | measured | $0 |
| Same, DERP `nyc` | 454 ms (471) | measured | $0 |
| **Pick the relay by latency** on the free map: DERP `tok` | **146 ms** (147) | measured | $0 |
| **Own DERP near the runner** (one small VPS) | about 12 to 15 ms | estimated: 2 × the 6 ms round trip to a host in the same metro (Cloudflare's PoP measured 6.2 ms) | about $5 to $6 per region (VPS list prices from memory, not checked today), plus patching and certificates. Three regions for travel: about $15 to $18 |
| **WebRTC direct**, browser and runner on the same machine or LAN | **0.3 to 0.5 ms** | measured, Chromium → Rust runner (`spike/src`) | $0 |
| WebRTC direct over the internet, both near each other | about the network round trip between them, typically 5 to 20 ms | estimated | $0 |
| **WebRTC via Cloudflare TURN**, both sides near one Cloudflare PoP | about 12 to 15 ms | estimated: 2 × 6.2 ms to `turn.cloudflare.com` (measured), plus relay time | $0 at our volume: the first 1,000 GB a month are free, then $0.05 per GB (owner's note, docs checked Nov 28) |
| Direct WebSocket or WebTransport to the runner | as direct | not built; see section 3 | $0, but needs certificates or open ports on the runner |

The relayed spike runs give an idea of what TURN adds. Through a local TURN server on loopback, the echo was still 0.4 ms over both UDP and TCP. The relay hop itself is therefore negligible, and the cost of a relayed path is the network distance to the relay.

### Expected TURN traffic

The largest single transfer is attach and replay, up to 8 MB (`crates/protocol/src/lib.rs:1220`). Polling and keystrokes are small.

A heavy month for one owner:

- 50 attaches a day × 8 MB × 30 days = about 12 GB.
- Only the relayed share of sessions counts, typically 10 to 20%.

That is about 1 to 3 GB, against 1,000 GB free. Even 100 users at this rate stay inside the free tier.

### Why the runner uses San Francisco when Tokyo is 2.5× faster

- The runner picks a region once, at its very first start, and writes it into `tailcat.key` as `region 302` (`crates/tailcat/go/tailcat.go:759-770`, `chooseRegion` at `:500-517`). That file was written Oct 18 and has not been re-picked since.
- Today none of the four relays answers STUN (0 of 8 replies each), so a pick falls back to HTTPS timing. Measured TCP connect times:

| Region | TCP connect |
|---|---|
| `tok` | 76 ms |
| `fra` | 172 ms |
| `sfo` | 191 ms |
| `nyc` | 230 ms |

- The browser has no say. Its WebAssembly build cannot probe regions (`pickregion_js.go` returns 0). It uses the single region in the runner's token, so both legs cross the Pacific.

A zero-code interim fix exists. Remove the `region 302` line from that runner's `tailcat.key` and restart the runner, and it re-picks; today that should be Tokyo, for 146 ms. Two cautions:

- That changes runner state. It is the owner's or the orchestrator's call, so I did not do it.
- The new token invalidates the old address, so web devices may need to reload their token.

## 2. What the spike proved

`spike/` holds three pieces:

- A Rust "runner" (`webrtc = "=0.21.1"`). It answers an offer over HTTP, standing in for the signaling worker, and splices each data channel to a TCP target (sshd or an echo server).
- A browser page that measures the path.
- A local TURN server (`pion/turn`) as a stand-in for Cloudflare.

### Runs with the browser on a relayed path

All with `policy=relay`, against local TURN, with the SSH test.

| Engine | TURN over | Opened in | Echo median | DTLS | SSH through the channel |
|---|---|---|---|---|---|
| Chromium 145 | UDP | 337 to 361 ms (2 runs) | 0.4 ms | 1.2, `ECDHE_ECDSA_AES_128_GCM` | sshd banner `SSH-2.0-OpenSSH_10.5` and its KEXINIT (curve25519, ssh-ed25519) |
| Chromium 145 | TCP | 345 to 352 ms (2 runs) | 0.4 ms | same | same |
| Chromium 156 | UDP / TCP | 427 / 353 ms | 0.4 ms | same | same |
| WebKit (Safari 27.2 engine) | UDP / TCP | 1,296 / 1,270 ms | 1 ms (WebKit rounds timers to 1 ms) | same | same |
| Firefox | not tested | n/a | n/a | n/a | n/a |

Firefox not tested: Playwright's Firefox exits with "Could not find profile folder" on this machine.

### Encryption

- **DTLS between the peers.** Each SDP carries an `a=fingerprint:sha-256` line, and both ends refuse a DTLS peer whose certificate does not match it. A TURN server, Cloudflare included, therefore relays only DTLS records.
- **SSH inside, unchanged.** The browser's SSH (`crates/web/src/ssh.rs`) still checks the pinned host key. The signaling worker carries the fingerprints, so a compromised worker could swap them and terminate DTLS itself. It would then see only SSH ciphertext, and it cannot pass the host-key check. At worst it can deny service or learn addresses.

### Problems the spike found

1. **The runner-side library cannot use TURN over TCP or TLS.**
   - webrtc-rs 0.21.1 skips `turns:` and `?transport=tcp` server addresses with a warning (`webrtc-0.21.1/src/peer_connection/transport/turn_relayer.rs:272-279`).
   - Browsers do support TURN over TCP and TLS. The measured runs above used it in Chromium.
   - So a browser on a UDP-blocked network (hotel, corporate) is fine. A runner on a UDP-blocked network has no path at all, where today DERP over HTTPS works.
   - Runner-side TURN over UDP did not connect against my loopback TURN server (2 tries, cause not found; it may be the test setup). It must be re-tested against Cloudflare.
   - Options:
     - Accept it: runners on home and office networks almost always have outbound UDP.
     - Add TCP TURN to webrtc-rs upstream: its TURN client crate (`rtc-turn`) already has a TCP transport type.
     - Use libdatachannel, which supports TURN over TCP and TLS, at the cost of a C++ dependency.
2. **The macOS Application Firewall blocks incoming UDP to unsigned binaries.** It is on here, auto-allowing signed software.
   - Over host addresses on this Mac, Chromium → Rust took 11 to 16 s to connect after the answer. It failed in 5 of 22 runs, and in all 3 runs with the runner bound to the LAN address.
   - Two Chromium peers on this Mac could not connect at all within 30 s.
   - Relayed runs, which need no incoming UDP to the browser, opened in 0.34 to 1.3 s in 9 of 9 runs.
   - Real Chrome and Safari are signed, and a Developer ID-signed `farcoolerd` should be auto-allowed. That must be confirmed with a signed build before Tailcat is deleted. If it fails, every Mac runner falls back to TURN, which still works but costs about 12 ms instead of under 1 ms.
3. **Address gathering is slow to finish.** With a STUN server configured, webrtc-rs needed 10.8 s to finish gathering here (some interfaces never answer). The product must use trickle ICE: send each address as it is found, rather than waiting for the full list. Signaling must carry those messages.

### Library facts

- **webrtc-rs `webrtc` 0.21.1.** Released Oct 11, 2026. It is a sans-IO rewrite on the `rtc-*` crates, with 4.2 M downloads in the last 90 days, MIT/Apache.
- **Size it adds:** the spike binary is 8.9 MB, or 6.9 MB stripped. It contains webrtc, tokio and serde. The daemon already links tokio, so the net addition is smaller (not measured).
- **Alternative:** `str0m` 0.26.0 (Nov 22). It is sans-IO and smaller, but has no built-in TURN client.
- **Tailcat's own WebRTC support** (tailscale/tailcat#4): open, 2 comments, last touched Sep 27, 2026. There is no timeline to wait for.

### Share of networks that need a relay

- Commonly cited figures for WebRTC services are 10 to 20% of sessions relayed. This is not measured here.
- This Mac's NAT is "endpoint-independent": three different STUN servers saw the same mapped port. That is the easy kind for hole punching.
- The runner should log the chosen path type (host, server-reflexive or relay) so that we measure our own rate.

## 3. Why the page cannot go direct today

- `services/web/src/client/tunnel.ts:1-20` loads Tailcat compiled for `GOOS=js` (`crates/web/build.sh:7,108`). Its only network primitive is a WebSocket to a DERP server.
- Upstream says so: "Browser traffic is relayed over DERP only, with no direct connections until WebRTC support" (tailcat README, issue #4).
- The CSP lists only the four DERP hosts (`services/web/wrangler.toml:69-74,119-120`, built in `services/web/src/headers.ts:52`).
- So every byte goes browser → DERP → runner and back.

Direct WebSocket or WebTransport from an https page instead of WebRTC:

- **`ws://` to a LAN address** is blocked as mixed content in Chrome, Firefox and Safari.
- **`ws://localhost`** is allowed in Chrome and Firefox, because loopback counts as secure. It only helps when the browser is on the runner itself.
- **`wss://`** needs a certificate the browser trusts for the runner's name. That means a public DNS name and an ACME certificate per runner, or a self-signed certificate the browser refuses.
- **Chrome's Local Network Access** asks the user before a public page reaches a private address.
- **WebTransport with `serverCertificateHashes`** accepts a self-signed certificate. Chrome supports it; Safari's support is recent and Firefox's partial. The runner still needs a reachable UDP port, so NAT is unsolved.

WebRTC needs none of this. The certificate is a throwaway pinned through signaling, and ICE does the NAT traversal. The engine support notes above are from my knowledge as of mid-2026 and were not re-checked today.

## 4. Threat model, compared with today

| | Today (Tailcat via DERP) | WebRTC + Cloudflare TURN |
|---|---|---|
| Who sees the runner's IP addresses | DERP operator (Tailscale) | Browser devices that signal it (LAN and public addresses in the address list) and Cloudflare when relayed. Signaling must be authenticated and limited to enrolled devices. |
| Listening ports on the runner | none (outbound only) | One ephemeral UDP port per session. It answers only ICE checks carrying that session's random credentials, and closes with the session. Nothing listens before a signaled offer. |
| Who can start a session | holders of an allowlisted node key (`crates/daemon/src/allowlist.rs:1-39`) | holders of a relay session for an enrolled device. The runner checks the device before answering. SSH keys still gate the shell. |
| What the relay sees | WireGuard ciphertext | DTLS ciphertext containing SSH ciphertext |
| Single third party | Tailscale's free DERP: best effort, revocable | Cloudflare: Workers for signaling, and TURN. The relay worker already runs on Cloudflare, so this adds no new vendor. |

What self-hosters lose: without a Cloudflare key, they get STUN (free and keyless at `stun.cloudflare.com`) but no TURN. Direct paths still work, and sessions that need a relay fail. They could run `coturn` and point the runner and page at it, with a setting like today's `FARCOOLER_DERP_MAP`.

## 5. Cutover tickets

Sized as S under a day, M one to three days, L more. The owner asked for a clean cutover with no migration, so there is no side-by-side period.

1. **Relay: carry WebRTC offers to the runner** (M). Smallest first ticket.
   - One Durable Object per runner. The runner keeps a hibernating WebSocket to it.
   - A browser with a valid session for an enrolled device posts its offer and trickled addresses; the runner's answer and addresses come back the same way.
   - It also mints short-lived Cloudflare TURN credentials from the key the orchestrator sets as a worker secret. The key is never sent anywhere else.
   - The relay has D1 today and no Durable Objects (`services/relay/wrangler.toml`), so this adds a binding and a migration.
   - Testable on its own with the spike's Rust peer and page.
2. **Runner: answer a WebRTC data channel and splice it to sshd** (M).
   - webrtc-rs 0.21.1, trickle ICE, servers from ticket 1, one peer connection per device session.
   - Logs the chosen path type and its round trip.
   - Acceptance includes a signed-build check that the macOS firewall admits it.
3. **Web: open the SSH stream over a data channel** (M).
   - Replace `tunnel.ts`'s Tailcat dial with an `RTCPeerConnection`. SSH in `farcooler_web.wasm` is unchanged, because it already takes a byte stream.
   - Drop `tailcat.wasm`: about 33 MB raw, 5.4 MB brotli, measured from `crates/tailcat/go` with `-s -w`.
   - CSP: replace the DERP hosts with `turn.cloudflare.com` and the signaling origin.
4. **CLI: `farcooler runner pipe` over WebRTC or plain SSH** (S to M). `crates/cli/src/runner_pipe.rs` is a Tailcat pipe that `~/.ssh/config` entries depend on (`apps/macos/Sources/FarCooler/SshConfig.swift`). It needs the Rust peer from ticket 2 in client mode, or removal together with the native apps.
5. **Delete Tailcat** (L, mostly mechanical; after 1 to 4 are live and measured):
   - `crates/tailcat`: 5,805 lines, 4,163 of them Go.
   - The Go toolchain: `actions/setup-go` in 6 workflows, 20 uses (`ci.yml` 9, `canary.yml` 5, `linux-binaries.yml` 2, `release.yml` 2, `tunnel-e2e.yml` 1, `web.yml` 1).
   - `scripts/build-tailcat.sh`, `scripts/build-tunnel-helper.sh`, `scripts/build-derper.sh`, `scripts/derp-hosts-check.py` and `.github/workflows/derp-hosts.yml`.
   - The DERP hosts in the CSP, and `FARCOOLER_DERP_MAP` (`crates/daemon/src/rendezvous.rs`).
   - Node keys on `authorized_keys` lines (`crates/daemon/src/allowlist.rs`).
   - `tunnel-e2e`, replaced by a WebRTC end-to-end test.
   - In all, 197 tracked files outside the crate mention Tailcat. Most are in the native apps and go with them under the web-first pivot.

### What would make me advise against deleting Tailcat

- A signed `farcoolerd` still gets its incoming UDP blocked by the macOS firewall, and only TURN works.
- The measured direct rate after tickets 1 to 3 falls under about 50%.
- A runner the owner cares about sits on a UDP-blocked network. That runner needs TURN over TCP or TLS on the runner side, which webrtc-rs lacks.

Any of these makes the case for keeping a TCP fallback before deleting the old path.

## 6. Not done, and why

- **Cloudflare TURN itself.** It needs the key. The orchestrator creates it, and builders get only short-lived credentials by file. None was provided, so the relayed numbers use a local TURN server and the Cloudflare figure is estimated from the measured 6.2 ms round trip.
  - To finish: run `spike/browsers.mjs` with `policy=relay&ice=[{urls:"turn:turn.cloudflare.com:3478?transport=udp",...}]` and with `turns:turn.cloudflare.com:5349?transport=tcp`.
- **The browser's own echo through today's path.** I followed the rule not to drive the live page. The native `tailcat` client stood in, so 360 ms is a floor.
- **Firefox.** Playwright's Firefox would not start here.
- **A real SSH login through the channel.** This Mac's sshd has no key for my user. Instead, the browser sent an SSH banner and received sshd's banner and key-exchange offer through the channel, which shows the byte stream is intact.

## Reproducing

```sh
cd reports/ov-1502/spike
CARGO_BUILD_JOBS=6 CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target" cargo build --release
# runner: answer offers on :8790, splice "ssh" channels to sshd
GATHER_MS=500 $FARCOOLER_SCRATCH/target/release/dcspike serve 127.0.0.1:8790 127.0.0.1:22
# browsers (needs a playwright-core whose browsers are installed; 1.64.1 matched here)
PLAYWRIGHT_CORE=<path>/playwright-core node browsers.mjs \
  'http://127.0.0.1:8790/?n=200&gather=1000&ssh=1&policy=relay&ice=<url-encoded RTCIceServer JSON>' chromium webkit
# today's path: native tailcat through one DERP region, direct paths off
TS_DEBUG_ALWAYS_USE_DERP=1 tailcat serve --key=r302 7777 &
python3 echo_rtt.py 100 -- env TS_DEBUG_ALWAYS_USE_DERP=1 tailcat <token> 7777
python3 stun.py stun.cloudflare.com tc302a.ipn.dev   # STUN round trips
```
