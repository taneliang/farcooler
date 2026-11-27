# ov-1499 · Web message feedback and idle requests

Enter now explains a pending writer claim, a read-only terminal, an earlier send still in flight, or a disconnected runner. The composer keeps the draft and its attachments until the runner accepts them. The raw terminal replays buffered keys if the claim succeeds and reports that they were unsent if the claim is denied. The Rust client core supplies the composer’s enabled state and explanation, and tells the raw terminal when to buffer, replay, or report unsent keys.

The runner records one `terminal.compose` outcome with elapsed milliseconds and the client from its hello. The success words are `sent` and `queued`. Refusals use the runner’s reason or error code. The record omits the message and attachments.

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

Runners that predate lease notifications still accept explicit claims and takeovers. Those runners can notify a displaced writer, while a viewer sees other lease changes the next time they act or reopen the pane.

## Regression evidence

The three Enter feedback assertions failed against the original chat renderer. The raw-terminal browser test also failed against the original renderer when the request was held, Enter was typed, and the runner denied the claim. Both Chromium suites now load the production WebAssembly bridge. They exercise all four composer states, both outcomes for buffered keys, and takeover in two tabs. The Rust tests cover compose log fields and notifications on release, claim, takeover, and disconnect.

Both Chromium suites, 93 affected web unit tests, 16 focused Rust tests, the TypeScript check, and the WebAssembly build passed on the follow-up. The original run also covered session closure and transport events. Its live lease and compose integration tests stopped during fixture setup. macOS rejected the Unix socket paths under the configured scratch directory, and the compose fixtures that start a pane could not bring up tmux there.
