# ov-1507 · Web workday browser walk

## Cause

Commit `f80d3d1c9` (ov-1484) moved every non-live terminal into **Unavailable Terminals**, a disclosure that starts closed. It also added the state badge to each link's accessible name. For the workday fixture, the link's name became `Lost Lost workday agent`. Opening the disclosure alone would therefore still leave the exact-name locator unmatched. The disclosure auto-opened when a lost terminal was already selected. Starting Codex in the workday walk left the running Codex terminal selected.

Commit `71b0ba9a2` (ov-1470) added the exact-name click and the two-runner CI step. The ov-1485 layout change at `bedde8580` changed the tabs and workspace controls. On that revision, the Web bundles and browser checks job reached tmux inventory recovery on build-box, then timed out after 30 seconds on the link. [GitHub Actions job log](https://github.com/taneliang/farcooler/actions/runs/45911805928/job/140229562752)

## Fix

Lost terminals appear under the live terminals with a visible **Lost** badge. The link keeps the terminal title as its accessible name and exposes the state as its accessible description. A **Recover** button beside the link uses the existing restart action for browsers with Control access. Recovering directly from the sidebar selects the returned terminal and lets Back return to the previous selection. The terminal link can still open the lost terminal's details. Ended terminals remain under the disclosure.

In the narrow desktop sidebar, the first version of the row broke the title into one word per line. Recover now moves below the title when the row is too narrow for both. At 320 px the two stay on one line.

The walk's locator is now scoped to the sidebar, and its recovery click uses the sidebar's Recover button. It has the same timeouts and steps as before.

## Verification

The unmodified `bedde8580` bundle reproduced the 30-second timeout at `csp-check.mjs:1320` in local Chromium with two isolated Rust runners. The checkout, Codex launch and tmux health checks had already completed.

Everything below ran on this branch rebased onto `29c41d4de`.

On the final Canary bundle (built at `b88d65e0b`), the two-runner workday walk passed three times in a row with no CSP violations. The third run saved the captures, taken before the walk selected the lost terminal. [320 px capture](workday-lost-terminal-320.png) · [1280 px capture](workday-lost-terminal-1280.png)

The Work browser suite passed 18 cases. At both 320 and 1280 px it checks that the lost row is visible with the disclosure closed, clicks Recover while another terminal is selected, checks which terminal the runner restarted, and checks that Back restores the previous selection. It also checks that a read-only browser sees the lost row and has no Recover button.

On the final bundle, these also passed: the Board, Conversation and Plan browser checks; the CSP walk with the fake daemon and CI's 100 ms tunnel delay (keystroke echo under load 19.6 ms at p95, limit 100 ms); the four-session CSP walk; and all seven split-pane sleep scenarios. The Canary bundle passed the size gate at 5,095,136 compressed bytes. The stable bundle was not built locally.

All 304 web tests, the web workflow tests and both TypeScript checks passed after the rebase, before the CSS-only row change.

## CI

**Not yet verified.** No CI run exists for this branch. The CI workflow runs on pushes to `main` and `landing/**`. In an earlier session, the approval system blocked a candidate push because it would publish the code and report to the public origin without explicit authorization. Nothing has been pushed. The failed run ([job log](https://github.com/taneliang/farcooler/actions/runs/45911805928/job/140229562752)) checked `bedde8580`, so rerunning it would test the old code. The acceptance item "passes on CI" stays open until this branch is pushed or landed.
