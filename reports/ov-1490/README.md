# ov-1490 · Joining with a browser and a terminal

A person or a runner can now join with a browser and the `farcooler` CLI. One thing still needs the Mac app: **getting Far Cooler onto a Mac** (see [What's still missing](#whats-still-missing)). Close that before the apps are removed (ov-1489).

## The matrix

"Before" is the tree without this ticket.

| Case | How it worked before | Native app needed | The web or CLI path now |
| --- | --- | --- | --- |
| **First device of a new account** | The browser alone could sign in, make a passkey and join the account's device list. To open a runner, the phone app scanned the browser's code (ov-771), or the person ran `farcooler enroll-browser` over an existing SSH login and carried the account ID, offer and reply by hand (ov-1470). | Phone app, unless they already had SSH access | **Add a Runner** is the first screen. It shows one `farcooler join <code>` command. The person runs it in the runner's terminal, checks the passkey, types `yes`, then matches one code in the browser. |
| **A second browser or phone, approved from an enrolled browser** | An enrolled browser could approve nothing. The phone or Mac app added it. `enroll-browser` also worked, with SSH access. | Phone or Mac app | New device: **Approve From Another Device** shows a QR code and **Copy Code**. Enrolled browser: **Devices**, then **Scan Its Code** or paste, check the passkey, choose runners, **Approve** with a verified passkey. Both screens show one code. A phone does this in its web browser. |
| **A new runner (Mac or Linux), from its own terminal** | The Mac app set up and paired a Mac. For Linux, `farcooler runner install you@box` installed it from another computer, and the app paired it with the account. | Mac app | `farcooler join` on that computer. It enrolls the browser on the runner, then adds the runner to the account once the person approves it in the browser. The runner and its workspaces appear with no reload; connecting takes one passkey touch. **Installing** on a Mac still needs the app. |
| **A runner the account has, not set up in this browser** | The browser didn't show it. | Phone app | Listed on Add a Runner and in the sidebar under **Not in This Browser**, with **Set Up**. Either run the command on it, or approve this browser from a device that opens it. |
| **Recovery with every enrolled device lost** | `enroll-browser` over a trusted SSH login. If only the browser's storage was lost and its passkey supports PRF, the sealed copy restored it (ov-779). | None with SSH access; otherwise a surviving app | The same `farcooler join`, at the runner's terminal as its Unix user. The runner refuses it from a pane, so an agent there can't type it. The relay gives its owner the same runner entry back with a new token, and refuses another account (`registered`). `enroll-browser` and the PRF restore still work. |
| **A device removed** | An app's device list, or `farcooler client revoke <id>` on the runner. A browser at Control was refused. `member log` didn't record approvals or `client revoke`. | Phone or Mac app, unless at the runner | **Devices**, **Remove**, **Confirm Remove**, verified by passkey. The runner closes the device's sessions: in the walk, the removed browser lost its connection 1.6 to 2.1 s after Confirm Remove. `farcooler client revoke` still works. |

## Who decides, and what changed in the rules

The rules are in the Rust core and the relay. The page asks them and draws the answer. Its Add Runner and Devices sentences also come from Rust (`crates/client/src/enroll_view.rs`).

**One rule is wider.** Before, a browser's passkey could do only defensive things. Now it can also approve another browser, by signing `client.enroll` with the passkey on its own `authorized_keys` line (`webauthn-line-es256`). The runner requires all of these:

- User verification (fingerprint, face or PIN), not only a touch; this app's origin; a challenge that works once; a signature over this exact request.
- The signer is the owner's or an Admin's browser, with Control.
- What it adds is a browser, at View or Control: no shell line, no `host_admin`, no teammate, no step-up key.
- The only other method this signature serves is `client.revoke`. Nobody can register `webauthn-line-es256` as a step-up key.

Every other path still needs a person at a trusted device or at a terminal on the runner.

**What the relay's ticket is worth.** It opens no runner. The browser's offer travels in the command the person copies, so the relay can't swap in another key. A ticket takes one reply within two minutes. The runner joins the account only after a signed-in approval and its own signature. The relay stores only hashes of tickets and claims.

**Audit log.** Each approval and removal made by `client.enroll` or `client.revoke` is in the runner's log (`farcooler member log`) as `device_added` or `device_removed`, with the device and who did it. Removing a member, and the sweep after it, keep their existing `removed` and `revoked` lines. The relay logs `runner.join_approved`, then `runner.registered` or `runner.token_rotated`.

## Risks worth a look

- **A pasted command is the classic trick.** If someone persuades a person to run the attacker's `farcooler join` code and type `yes`, the attacker's browser gets Control. The prompt says what it grants and shows the passkey. The device's name in that prompt is chosen by whoever made the code. Anyone who can get a command run on the computer already has this much.
- **Removing a device leaves what it added.** A stolen, unlocked browser whose thief passes user verification could add another browser. The web's Remove takes only the device you pick. The new one stays in the list until it is removed too. `farcooler client revoke --with-added` takes both. I suggest a follow-up that offers this in **Devices**.
- **Removal reaches connected runners.** The browser removes the device from every runner it is connected to, and names the ones it isn't. Those keep letting it in until it's removed there.

## Open decisions, and where they bite

I decided neither.

- **ov-778 (teammates enroll with Control).** A Member's browser can't approve even their own second browser, or remove a device (`not_admin`). The web Worker posts an approval only to the session's own account, so a browser can't approve a teammate's. Adding a teammate still depends on the phone's flow, and the teammate comparison screen still says "phone". **Don't use `farcooler join` for a teammate:** run on your runner, it enrolls their browser as one of yours. The CLI prompt doesn't show which account the code is for. This blocks removing the phone app for teams.
- **ov-779 (the node key after storage eviction).** With PRF, a surviving passkey restores everything. Without it, an evicted browser has to be added again with `join` or an approval. If that takes a new passkey, the old line stays in **Devices** until someone removes it. A runner may also answer `device_taken` for a device it already knows or removed; the page then asks for a new passkey.

## For the owner

- **No secrets to write, and no flags flipped.**
- **Deploy the relay first.** Apply `services/relay/migrations/0034_join_tickets.sql` (it only adds a table), then deploy the relay, then the web Worker. If the web Worker goes first, Add Runner says it couldn't make a command. I deployed nothing. No other branch has a `0034`.
- **Security review of new Worker routes** (`api.ts` asks for one, ov-810): `directory/teams/runners`, `ceremony/reply/post`, `join/ticket`, `join/approve`. Canary has `WEB_CLIENT = "everyone"`, so any signed-in person can reach them there. Stable has it off. The relay's `join/check`, `join/reply` and `join/register` take no session. A ticket or claim guards them, and they are rate-limited by address with the existing `CEREMONY_REPLY_LIMIT` binding.
- **Runners need this build** to accept an approval or a removal from a browser. The page says an older one needs an update.

## What's still missing

- **Installing without an app.** `farcooler join` needs Far Cooler already on the computer. On Linux, `farcooler runner install` does it from a computer that has the CLI, or you can build from source. On a Mac, [docs/runners.md](../../docs/runners.md) says setup means running the Mac app once. `join` starts a daemon if none is running, but doesn't set it to start at login or put it where the fence's forced command looks (`~/.local/bin`). This needs its own ticket under ov-1489.
- **Camera scanning isn't tested.** The browser walk pastes the code. **Scan Its Code** uses the page's existing QR scanner.
- **Old native codes are refused here.** A native phone or Mac app's code isn't a browser's (`not_a_browser`).

## After the rebase onto 8d8a2f2b7

The landing base moved the web app to a new React sidebar and design system (ov-1510). The three conflicts were in `app.css`, `app.ts` and `index.html`. I kept the base's shell and moved this ticket's controls into it:

- **Add Runner** and **Devices** are in the sidebar under the runner's name. **Not in This Browser** is below them. At phone width, all three are behind the button that names the runner.
- **Add a Runner** is a shell screen, like the other screens before a runner is connected.

The new design allows one filled button per screen, and the walk now checks that on every screen it captures. That check and the new captures led to four fixes:

- **Approve** in Devices is outlined. It was a second filled button, over **Recover Terminal**.
- When the connection is lost, only **Reconnect** is filled. A removed browser also showed a filled **Recover Terminal**.
- At 320 wide, the shell hid the **Not in This Browser** heading. It now shows.
- At 320 wide, the passkey to compare, and then the code, appeared below the fold in Devices. The sheet now scrolls to each. The walk's new check failed without the scroll.

## Verification

All of this ran after the rebase.

**Browser walk against a real runner** (`csp-check.mjs --join`, in `join-walk.mjs`): the real CLI, daemon, tmux and an OpenSSH server running the forced command the fence writes, in Chromium with virtual passkeys. All six steps passed with no CSP violations: first screen and `join`; the runner not set up here; a second browser context approved by the first; removal; the audit log; recovery with no device left. The relay is stubbed in this walk. Its rules are tested against the real migrations in `services/relay/test/join.test.ts`.

I didn't run the walk against a build without this change. It can't pass there: that tree has no Add Runner screen, Devices sheet or `farcooler join`.

I added this walk to the Web job in CI. It has run only on macOS, so its first Linux run is still to come.

**The workday walk fails at its last step, on the landing base too.** CI runs it. It waits for **Checking Here** under the Plan's Landing heading, and it never appears. I built `8d8a2f2b7` by itself and ran its own walk: it stopped at the same step, after passing the same earlier ones. As far as I read it, the web Plan tab doesn't pass the engine's workflow data to the Plan view, so engine landing groups have nothing to draw. I didn't confirm that cause, and I didn't change it. It needs its own ticket. Until it's fixed, that CI step stays red. I moved the join walk ahead of it, so the join walk still runs.

On this branch every earlier step of that walk passes. I changed one: a runner you've just added is now the one offered next, so the walk connects to it, then switches runners twice.

| Acceptance line | Test |
| --- | --- |
| Recovery from the terminal, refused without it | Walk step 6; `with_every_device_gone_the_runners_socket_enrolls_a_browser_and_logs_it` (a removed device is refused); `admin_requests_from_inside_a_pane_are_refused`; relay `join.test.ts` |
| Removed device loses access at once | Walk step 4; `browser_approval_tests.rs` |
| Approval limits | `browser_approval_tests.rs`. Before the rebase, with the browser's limit disabled, `a_browsers_approval_adds_only_a_browser_at_control_or_below` failed, as it should. |
| Same runner joins again | Relay test. Before the rebase, with the owner refused, it failed (`409` instead of `200`). |

| Suite | Result |
| --- | --- |
| Relay, all | 849 passed |
| Web unit tests, all; Needs You and workflow tests; both TypeScript checks | 271 and 50 passed; workflow passed; clean |
| Web crate | 92 passed |
| Ceremony and fence crates | 184 passed, 1 ignored |
| Client crate, library | 429 passed, 2 ignored |
| Daemon: step-up, approval, join, enrollment, people, sweep, passkey and pane-refusal filters | 300 passed; nothing left behind |
| CLI browser, join, member and log filters | 13 passed |
| `clippy -D warnings`: ceremony, fence, client | clean (the daemon and CLI weren't rerun; the rebase didn't conflict in Rust) |
| Shared core, daemon layers, file size, visual tokens, copy, doc comments, migrations, web tree, capability names, plan layer, proto | passed |
| Browser checks: shell, plan render, chat, chat files, work, board | passed |
| Main browser walk (`--fake-daemon`) | passed with no CSP violations |
| Canary bundle size gate | 5,305,972 bytes compressed, within limits |

I didn't rerun the full daemon suite. The browser checks, both walks and the size gate ran after the four fixes.

## Captures

Every flow at 320 and 1280 wide, in [captures/](captures/). Each name is `join-<screen>-<width>.png`.

| Flow | Screens |
| --- | --- |
| First runner | `first`, `first-compare`, `runner-added` |
| Not set up here | `unenrolled-sidebar`, `unenrolled-add-runner` |
| Approve a device | `approve-new-device`, `devices`, `approve-confirm`, `approve-code`, `approve-new-device-compare`, `approved-browser-open` |
| Remove a device | `remove-confirm`, `removed`, `removed-browser` |
| Recovery | `recovery`, `recovery-compare`, `recovered` |

## Reproduce the walk

From `services/web`, in a tree with no uncommitted changes to tracked files:

```sh
export CARGO_TARGET_DIR="$FARCOOLER_SCRATCH/target"
../../crates/web/build.sh --channel canary --out "$FARCOOLER_SCRATCH/wasm"
WEB_WASM_DIR="$FARCOOLER_SCRATCH/wasm" ./build.sh --channel canary
export FARCOOLER_CHANNEL=canary   # later builds without it break the walk: the CLI refuses the page's code
cargo build --locked --manifest-path ../../Cargo.toml -p farcooler-daemon \
  --features tailcat-helper --bin farcoolerd --example web_workday_runner
cargo build --locked --manifest-path ../../Cargo.toml -p farcooler-cli --bin farcooler
node test/browser/csp-check.mjs --join \
  --workday-runner "$CARGO_TARGET_DIR/debug/examples/web_workday_runner" \
  --workday-cli "$CARGO_TARGET_DIR/debug/farcooler"
```
