# ov-1534: the relay carries WebRTC offers to the runner

Ticket 1 of the Tailcat-to-WebRTC cutover (reports/ov-1502/report.md §5). Nothing deployed, no secret created or read, no flag changed. Measured on this Mac on Nov 29, 2026.

## What landed

- `services/relay/src/signal.ts`: the routes and the `Signal` Durable Object, one per runner. The relay README's "WebRTC signaling" section lists the routes.
- `services/relay/src/turn.ts`: TURN credentials minted per session for 3600 s. With no key, the relay hands out STUN only and says so.
- `wrangler.toml`: a `SIGNAL` binding in all four environments and one inherited migration (`v1-signal`, `new_sqlite_classes`). Both are additive. `wrangler deploy --dry-run` passed for stable, preview, canary and local.
- The spike gained a `dcspike relay <ws-url> <token> <ssh-target>` runner, and its page gained `?relay=` mode, to test the relay end to end.

## Contract additions (for the orchestrator)

Two fields beyond the contract in the ticket:

1. **`offer.device`.** The offer the relay forwards to the runner carries `device {id, account, fingerprints}`. These are the directory device the relay checked, its account, and its keys' `SHA256:` fingerprints, so the runner can check the device itself by matching them against `authorized_keys`. The ov-1535 session was told this.
2. **`ice_servers.stun_only`.** It is `"no_key"` or `"refused"` when there is no TURN credential, and absent otherwise.

## Who may signal

`open` and `send` refuse with a single 404 unless all of these hold:
- the session's account owns the device;
- the device is trusted or certified;
- one of its fingerprints is in the runner's reported inventory;
- the account is a member of the runner's team.

`poll` answers only the account that opened the session. The runner's socket is bound to the runner row of its daemon token.

Mutation check: each of these 14 checks was removed in turn, and the suite went red every time, each time from its own test:
- the four enrollment clauses;
- the session requirement;
- routing by token, routing by runner;
- the poll owner and the send owner;
- the rule that runners can't hand out servers;
- STUN only without a key;
- the short TTL;
- logging no error body;
- no mint before admission.

The first pass left three checks uncovered, because a different clause refused those cases first. Three new tests isolate them:
- a teammate naming a colleague's device;
- a key that is on a sibling runner in the same team;
- an enrolled teammate naming someone else's session.

## End to end

`node reports/ov-1534/e2e.mjs <bundle> <dcspike> <playwright-core> [relay|all]` runs:
- the bundled worker in Miniflare (real D1 migrations, real Durable Object);
- WorkOS's JWKS and Cloudflare's credential endpoint faked by Miniflare's outbound fetch;
- `dcspike relay` as the runner;
- the spike page in Chromium 145 as the browser.

| Path | TURN mint | Channel open | Echo median | Candidates trickled, browser → runner / runner → browser | sshd through it |
|---|---|---|---|---|---|
| Both relayed via local TURN (`policy=relay`), 2 runs | 1 per session, ttl 3600, token only in the Authorization header | 1.4 to 1.6 s | 190 ms | 3 / 1 | banner `SSH-2.0-OpenSSH_10.5`, KEXINIT with curve25519 and ssh-ed25519 |
| Direct, no TURN key (`stun_only: "no_key"`), 1 run | none | 39 ms | 0.4 ms | 2 / 3 | same |

Both runs also tested refusal: a stranger's `open` naming the runner and the device got `{"error":"not found"}`. The account token never appeared in the runner's output.

**The 190 ms relayed echo is a finding for ticket 2, not for signaling.** It held in both runs. It happens only when both peers relay through the spike's loopback TURN server, and webrtc-rs logged a "TURN transaction timed out" each time. In ov-1502 the runner was not relayed, and the echo was 0.4 ms with the browser relayed. Re-measure it against Cloudflare TURN once the key exists.

To run: start `reports/ov-1502/spike/turnsrv` for `relay` (it needs a `go.mod`, as `go mod init turnsrv && go get github.com/pion/turn/v4`). Build the spike with `CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target"`. Bundle the relay with `npx wrangler deploy --dry-run --outdir <bundle>`.

## Notes

- Standalone workerd refuses this worker's entry module, main's bundle included, because `index.ts` exports constants for its tests. The harness re-exports only `default` and `Signal`. Production deploys take the module as it is today.
- The test pool can't pop isolated storage under a SQLite Durable Object holding a WebSocket. So `test/signal.test.ts` runs alone with isolation off (`vitest.signal.config.ts`), and `npm test` runs both suites.
