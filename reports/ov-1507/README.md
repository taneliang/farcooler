# ov-1507 · Web workday browser walk

## Cause

Commit `f80d3d1c9` (ov-1484) moved every non-live terminal into **Unavailable Terminals**, a disclosure that starts closed. It also added the state badge to each link's accessible name. For the workday fixture, the link's name became `Lost Lost workday agent`. Opening the disclosure alone would therefore still leave the exact-name locator unmatched. The disclosure auto-opened when a lost terminal was already selected. Starting Codex in the workday walk left the running Codex terminal selected.

Commit `71b0ba9a2` (ov-1470) added the exact-name click and the two-runner CI step. The ov-1485 layout change at `bedde8580` changed the tabs and workspace controls. On that revision, the Web bundles and browser checks job reached tmux inventory recovery on build-box, then timed out after 30 seconds on the link. [GitHub Actions job log](https://github.com/taneliang/farcooler/actions/runs/45911805928/job/140229562752)

## Fix

Lost terminals appear under the live terminals with a visible **Lost** badge. The link keeps the terminal title as its accessible name and exposes the state as its accessible description. A **Recover** button beside the link uses the existing restart action for browsers with Control access. Recovering directly from the sidebar selects the returned terminal and lets Back return to the previous selection. The terminal link can still open the lost terminal's details. Ended terminals remain under the disclosure.

## Verification

The unmodified `bedde8580` bundle reproduced the 30-second timeout at `csp-check.mjs:1320` in local Chromium with two isolated Rust runners. The checkout, Codex launch and tmux health checks had already completed.

The fixed Canary bundle passed three consecutive runs of the same two-runner workday command with no CSP violations. A fourth run captured the lost terminal and the Recover button before the walk selected the lost terminal. [320 px capture](workday-lost-terminal-320.png) · [1280 px capture](workday-lost-terminal-1280.png)

The Work browser suite passed 18 cases. At both 320 and 1280 px it verified the lost row was visible with the disclosure closed, clicked Recover while another terminal was selected, checked which terminal the runner restarted, and verified that Back restored the previous selection. It also checked that a read-only browser could see the lost row and had no Recover button.

The Board, Conversation and Plan browser checks passed. All 304 web tests, the web workflow tests and both TypeScript checks passed. The Canary bundle passed the size gate at 5,091,790 compressed bytes.

The full CSP walk with the fake daemon and CI's 100 ms tunnel delay passed without violations. Its workload keystroke echo was 21.3 ms at p95 against a 100 ms limit. The four-session CSP walk and all seven split-pane sleep scenarios also passed.

GitHub Actions verification of the fix is pending. The existing Web bundles and browser checks job runs the two-runner walk on the landing candidate.
