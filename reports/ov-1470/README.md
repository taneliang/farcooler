# ov-1470 · Remote browser workday

The browser can switch between enrolled runners, display connection and daemon health, and resume a terminal after a passkey reconnect. The Rust runner’s terminal writer lease keeps a second connection read-only while a writer holds the pane. A tab keeps its writer identity across tunnel reconnects. The Mac input pipe can explicitly take over a terminal, and the browser shows the viewer status while preserving the draft. Plan shows the runner’s landing phase, and Start Work opens the selected checkout’s committed branch diff.

## First browser on a runner

1. Open the web client, sign in, make a passkey, and expand **Approve from the runner CLI**.
2. Use an existing, trusted SSH login on the runner. Run the command below with the account ID displayed by the page. Use the SSH host public key configured for the runner’s SSH server. The example uses its Ed25519 host key.
3. Paste the browser offer, compare the passkey fingerprint, and enter `yes`. Paste the sealed reply into the browser, compare the CLI and browser codes, and confirm. Select **Connect** and use the passkey.
4. To add a second runner, select **Add Runner** and repeat. **Switch Runner** opens the selected runner’s own workspace and connection.

```sh
farcooler enroll-browser --account '<account-id>' --user "$(id -un)" \
  --label '<runner name>' --host-key "$(cat /etc/ssh/ssh_host_ed25519_key.pub)"
```

The enrollment grants Control to that passkey. It requires a runner with browser-passkey support and a working Tailcat route. The same procedure recovers access with a new passkey after local browser storage is lost. The browser also retains its existing passkey recovery flow.

## Script boundaries and cutover handoffs

| Step | Surface used by the script | Owner trial notes |
| --- | --- | --- |
| Browser sign-in, passkey and runner approval | Browser and `farcooler enroll-browser` on each runner | The CLI handoff is needed for each grant in a fresh profile. An existing trusted SSH login is the starting point. |
| Runner switch, repo and checkout selection, start Codex, recover a lost agent | Browser | The fixture pre-registers a repository, sets up a checkout and stands in for Codex. The owner trial should use the owner’s configured repositories and actual agent sign-in. |
| Runner health during an active session | Browser | The fixture makes tmux inventory reads fail after sign-in, then restores them. The browser shows degradation and recovery while the connection stays up. |
| Competing writer, connection loss and resume | Browser and CLI fixture | A second connection can read the conversation and raw terminal while the first holds the writer lease. Its composer, image picker and Stop are disabled. The cross-client check uses `farcooler terminal input` and `farcooler terminal screen` to verify explicit takeover through the Mac’s input channel. These are fixture controls. The fixture closes a loopback tunnel, and one passkey touch reopens the selected runner and terminal. |
| Needs You attention and response | Browser | The fixture places a decision on the runner. The browser displays the attention count and records the answer through the ordinary task RPC. |
| Diff and landing | Browser | The fixture commits one file and supplies a landing in local checks. The browser reads the committed diff and follows the landing from local checks to CI. The fixture sends no live push to main. The Board’s Changes flow from ov-1458 also supports inline review and approval on tickets. |
| Web Push | Browser opt-in, service worker, relay | Web Push needs a relay-paired runner and configured VAPID keys. The tests use a fake push service. The owner trial needs a live delivery check with the tab closed. |

The test harness substitutes WorkOS sign-in and the Tailcat network with an admitted test account and a loopback splice. It starts two disposable Rust services, uses real OpenSSH passkey authentication and the real CLI, and exercises the normal runner RPCs, Git and tmux. The only operator handoff is CLI enrollment. Repo fixture preparation, writer contention, tunnel loss and the landing fixture are test controls. The script never invokes the Mac app.

The Chromium workday is part of the **Web** CI check. It timed out waiting for degradation on the original browser bundle. The relay test uses a fake push provider to verify that delivery follows account opt-in and stops after opt-out. It uses a colon-delimited FCM subscription token, which received a 400 before the endpoint validation fix. The browser adapter test simulates a subscription left behind after the relay forgets it. One opt-in click now replaces the old endpoint and enrolls the new one.

## Verification on landing base `7a08f9c10`

The Chromium workday passed with two isolated Rust runners and no CSP violations. A fresh browser enrolled both runners through the CLI. It switched runners, selected the checkout, started Codex, recovered a lost terminal, and showed tmux degradation and recovery in the same connection. A competing tab could view the conversation and raw terminal while the writer held input. The native CLI input pipe took over explicitly and the browser kept its draft. After a tunnel drop, one passkey touch restored the selected terminal and writer access. The browser then answered Needs You, read the committed diff, and watched the landing move from local checks to CI.

The web suite passed 305 tests, the relay suite passed 843, and the Rust Web suite passed 83. The Rust client and CLI suites and all three focused daemon writer tests passed. The Chromium Work, Board, Changes review, Plan, Conversation, and split-pane sleep checks passed. The full Chromium security check passed on rerun with no CSP violations and a 19.0 ms p95 keystroke echo against a 100 ms limit. Its first run measured 111.7 ms. TypeScript, web workflow tests, and shared-core lint passed. The Canary bundle passed the size gate at 5,053,935 compressed bytes.

## Verification before the landing-base rebase

The branch replayed onto `541e48047b5e14f26b92a482d3573e96a92f96c3` without conflicts. The Chromium Work, Board, Plan, split-pane sleep, passkey recovery, and sign-out checks passed in the initial verification. After the FCM fix, the Chromium workday passed again against two disposable Rust runners with no CSP violations. The rebuilt canary bundle passed the size gate at 5,030,824 bytes compressed.

The web suite passed 304 tests, the relay suite passed 843, the CLI passed 639, the Rust web suite passed 83, and the native Rust client passed 410. The focused daemon RPC and session suites passed 49 tests, including competing browser writers and native input with a browser connected. The native Swift conversation test passed against the real CLI and daemon. TypeScript checks passed, and the shared-core lint passed without growing its baseline.

The opt-in native Release performance test fed and drew 128–146 terminal updates in each of five runs while the conversation streamed. Two runs exceeded its 35 ms stall limit, with a worst stall of 97 ms. The performance test failed on that timing assertion.

After the Web Push subscription recovery fix, the Chromium workday passed again on two disposable runners with no CSP violations. The canary bundle passed the size gate at 5,027,860 compressed bytes. The follow-up passed 305 web tests, 843 relay tests, 83 Rust web tests, 639 CLI tests, 410 native Rust client tests, and 28 focused daemon RPC and session tests. The web workflow tests, TypeScript checks, and shared-core lint also passed.

## Reproduce the Chromium workday

Run these commands from `services/web` in a clean checkout with Chromium and the web build tools installed.

```sh
export TMPDIR="$FARCOOLER_SCRATCH"
export CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target"
export GOCACHE="$FARCOOLER_SCRATCH/go-cache"
export FARCOOLER_CHANNEL=canary
../../crates/web/build.sh --channel canary --out "$FARCOOLER_SCRATCH/wasm"
WEB_WASM_DIR="$FARCOOLER_SCRATCH/wasm" WEB_DIST_DIR="$FARCOOLER_SCRATCH/dist" \
  ./build.sh --channel canary
cargo build --locked --manifest-path ../../Cargo.toml -p farcooler-daemon \
  --features tailcat-helper --bin farcoolerd --example web_workday_runner
cargo build --locked --manifest-path ../../Cargo.toml -p farcooler-cli --bin farcooler
node test/browser/csp-check.mjs \
  --dist "$FARCOOLER_SCRATCH/dist" \
  --workday-runner "$CARGO_TARGET_DIR/debug/examples/web_workday_runner" \
  --workday-cli "$CARGO_TARGET_DIR/debug/farcooler"
```

## Web Push configuration

Apply the relay migration and configure a P-256 VAPID keypair per relay environment. Set `WEB_PUSH_PUBLIC_KEY` to the unpadded base64url of the uncompressed public point and install the private JWK as the `WEB_PUSH_PRIVATE_JWK` secret. The browser’s **Web Push** control appears when the relay has a key and the browser supports subscriptions. Delivery uses a short-lived, empty wake-up message. The service worker opens Needs You, where the signed-in browser reads the item normally. Signing out everywhere removes the account’s relay subscriptions. On the next sign-in the browser checks the relay’s enrollment and asks for opt-in again.
