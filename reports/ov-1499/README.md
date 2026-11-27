# ov-1499 · Web message feedback and idle requests

Enter now explains a pending writer claim, a read-only terminal, an earlier send still in flight, or a disconnected runner. The composer keeps the draft and its attachments until the runner accepts them. The raw terminal replays buffered keys if the claim succeeds and reports that they were unsent if the claim is denied. The Rust client core supplies the composer’s enabled state and explanation, and tells the raw terminal when to buffer, replay, or report unsent keys.

The runner records one `terminal.compose` outcome with elapsed milliseconds and the client from its hello. The success words are `sent` and `queued`. Refusals use the runner’s reason or error code. A compose the runner drops unanswered, 30 seconds after its connection ended, is recorded as `abandoned`. The record omits the message and attachments.

## Idle requests per minute

This is the configured steady-state rate on the landing base for one connected runner with the Terminals page visible, a selected workspace, and no changes. The plan fits one response. Counts exclude initial reads, event-triggered reads, transcript follow requests, and connection probes.

| Runner RPCs | Before | After |
| --- | ---: | ---: |
| Needs You and work inventory | 60 | 60 |
| `host.health` | 12 | 12 |
| `plan.get` for orchestrator discovery | 12 | 0 |
| `terminal.writer` status, per visible pane | 60 | 0 |
| **Total with one pane** | **144** | **72** |
| **Total with two panes** | **204** | **72** |

The before rates come from the five-second page refresh and the one-second delay after each lease response. Round-trip time can lower the actual lease rate. On a runner with workflow support, discovery also made at least one workflow read per plan refresh, adding another 12 requests per minute before. The page now gets the orchestrator from its existing work inventory. Chromium advanced an idle pane through 60 seconds and observed zero lease status calls. A lease event triggers a status read for the pane it names.

A lease read that fails is tried again after 2 seconds, doubling to 30 seconds, until one answers. Without this, removing the poll would have left a pane read-only after a single timed-out read. A failed takeover is not repeated: the retry only reads. These retries happen only after a failure, so the idle counts above don’t include them.

Runners that predate lease notifications still accept explicit claims and takeovers. Those runners can notify a displaced writer, while a viewer sees other lease changes the next time they act or reopen the pane.

## Regression evidence

The three Enter feedback assertions failed against the original chat renderer. The raw-terminal browser test also failed against the original renderer when the request was held, Enter was typed, and the runner denied the claim. Both Chromium suites now load the production WebAssembly bridge. They exercise all four composer states, both outcomes for buffered keys, and takeover in two tabs. The Rust tests cover compose log fields and notifications on release, claim, takeover, and disconnect.

A later pass added three tests, each run red first or checked by putting the old behavior back: the dropped compose (no row before the fix), the failed lease read (the pane stayed Viewer), and the four composer assertions plus the raw-key refusal (all failed with the messages removed).

That pass also restored the earlier handling of a grant with no writer lease, so the runner’s own refusal is shown instead of a read-only notice with no button. It fixed two checks CI would have failed (`daemon-layers.py` and `shared-core-lint.py`) and removed the unused plan-based orchestrator lookup.

The Chromium chat, workspace and board suites passed, along with all 254 web unit tests, both TypeScript checks and the focused Rust tests for the client core, the web crate and the runner.

On macOS the five live daemon cases for lease takeover, input admission, the shared stdio writer and compose dispatch passed with the ticket scratch directory.

On the same macOS machine, the full page walk through a scratch sshd, the Chromium sleep and reconnect walk, and the two-runner workday walk passed against the canary bundle. The workday walk covers a native input pipe taking over from the browser on a real runner. Two expectations in those walks were out of date. Billing’s fixture now names its orchestrator, so that pane leads its sidebar. The lost agent’s link name includes its state badge, which was already true on the landing base.

## Rebase onto `d9c0306c2`

The only textual conflict was in `.github/workflows/ci.yml`, where ov-1509 added `chat-files-check.mjs` after the chat check. Both steps are kept. The new check also bundles `chat-page.ts`, which loads the WebAssembly bridge on this branch, so it failed to bundle after the rebase (`Could not resolve "fc-web"`). It now loads the bridge the same way `chat-check.mjs` does, and its CI step gets `WEB_WASM_DIR`.

After the rebase, these passed on macOS: both TypeScript checks, the 254 web unit tests, the 50 Needs You tests and the workflow fixture test; the chat, sent-files, plan, work and board Chromium suites; the canary bundle with its size gate, the CSP walk against the fake daemon, the Chromium sleep walk and the two-runner workday walk; all `crates/web` tests; the `farcooler-client` library tests (422 passed, 2 ignored); the selected runner library tests (178 passed, 1 ignored); the two live lease cases, the two shared stdio writer cases and `agent_rows_over_every_transport` (6 passed, 1 ignored). The stable bundle, the Go wasm test and the full runner test suite weren't run.

## Landing gate repair

The gate failed with `ENOENT … wasm-canary/farcooler_web.wasm`. CI builds that directory with Linux-only tools, so the Mac gate skipped the build. It still ran the browser checks that read the directory, because they name it only in `env:`, which the skip rule doesn't read. Before this branch, those checks didn't need the module.

The gate now builds the module itself before the first check that reads it. `crates/web/test-module.sh` installs the `wasm-bindgen` version that `Cargo.lock` names, under the target directory, then runs `build.sh --module-only`. That mode skips `wasm-opt` and Tailcat, and its output is never deployed. In the passing run the step took 79 seconds, including the install. A new gate self-test failed on the three affected checks before the fix. CI's own steps are unchanged.

The failed run had also stopped before the Rust tests finished. Running them found a second failure: the CLI test that requires a decision for every event kind didn't cover `terminal_writer_changed`. The CLI never asks for that event, so it's now listed as having no reader.

On macOS, through `scripts/landing-gate`: `web` passed at `77492dd13`; `rust` and its lints passed at `83016911d` (7,616 nextest tests, then the doc tests); every `wire` and `doc-comments` step passed at `77492dd13`. One earlier Rust run reported `ov1427_restart_reaps_a_native_owner_that_survived_the_runner` as flaky (it passed on its rerun); no ov-1499 commit changes that test or the code it covers (the ov-1475 commits in this range do change that code). The final Rust run had no flaky tests. `relay`, `site`, `tunnel` and the two Swift families weren't rerun; they passed in the failed landing run.

With `CARGO_TARGET_DIR` shared across the whole gate, one `lane-gate.py --self-test` case failed because its fixture's `cargo test` didn't start in time. It passed without the variable.

`scripts/landing-policy-test.py`, run by hand with `FARCOOLER_BASE` set, reports two failures and one error, on this branch and on the base alike. Under `lane-gate.py --self-test` it passes.

## Rebase onto `edf1f42fa`

Two files conflicted.

- `crates/web/src/wire.rs`: only the `use` lines conflicted. ov-1513 added `link_ending` and this branch added `event::Payload`, so both stay. The rest merged on its own: ov-1513's foreground check and ending reasons run beside the writer-event listener.
- `services/web/test/browser/csp-check.mjs`: ov-1507 rewrote how the workday walk finds the lost terminal. Main's version is kept. It replaces this branch's name fix.

The idle request counts above still apply. These commits add no periodic request to the web client.

On macOS with Node 24, these passed on the rebased code: both TypeScript checks, the 263 web unit tests, the 50 Needs You tests and the workflow tests; the chat, sent-files, plan, work and board Chromium suites; the canary bundle with its size gate and self-test, the CSP walk against the fake runner, the Chromium sleep walk and the two-runner workday walk; all `crates/web` tests; the client (423 passed, 2 ignored), protocol (73) and transport (23) library tests; the CLI event tests (10); the selected runner library tests (231 passed, 2 ignored); the `rpc_over_socket` (90), `stdio_transport` (13), `agent_rows_over_every_transport` (6 passed, 1 ignored) and relay hang-up (1) suites; the web-only `wasm32` check of the client core; `daemon-layers.py`, `shared-core-lint.py`, and `lane-gate.py` with `--quick-only` and with `--clippy-only`.

The full Rust suite, the stable bundle, the Go wasm test and the Swift families weren't run.

## Rebase onto `022667bfb`

The five new commits are ov-1510's React shell. Four browser checks conflicted: `chat-check.mjs`, `chat-files-check.mjs`, `work-check.mjs` and `csp-check.mjs`. The first three now use ov-1510's shared styles and HTML (`scripts/styles.mjs`), keep its `--measure` option, and load the WebAssembly bridge as this branch did. `csp-check.mjs` uses ov-1510's selectors and still expects Billing's orchestrator first.

The `crates`, `proto` and `scripts` trees and `Cargo.lock` are identical to the previous candidate's, and the new commits add no timer or periodic request to the web client. The idle request counts above still apply.

The workday walk then failed on a line this branch hadn't touched. ov-1510 replaced the **Switch Runner** `<select>` with its own picker, but the walk on `022667bfb` still called `selectOption` on it. The walk now opens the picker and selects the option. This is a test-only change.

On macOS with Node 24, the local gate's `web` family passed. It covers both TypeScript checks, the web unit and workflow tests, the `crates/web` tests, the Go wasm test, the client's `wasm32` check, and the chat, sent-files, plan, shell, work and board Chromium suites. `lane-gate.py` passed with `--quick-only` and with `--clippy-only`. By hand, the canary bundle passed its size gate, and the CSP walk against the fake runner, the Chromium sleep walk and the two-runner workday walk passed.

The other gate families, including the full Rust suite and the Swift families, and the stable bundle weren't run.
