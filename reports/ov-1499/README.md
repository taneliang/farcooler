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
