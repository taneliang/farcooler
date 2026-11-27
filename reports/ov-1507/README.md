# ov-1507 · Web workday browser walk

## Cause

Commit `f80d3d1c9` (ov-1484) moved every non-live terminal into **Unavailable Terminals**, a disclosure that starts closed. It also added the state badge to each link's accessible name. For the workday fixture, the link's name became `Lost Lost workday agent`. Opening the disclosure alone would therefore still leave the exact-name locator unmatched. The disclosure auto-opened when a lost terminal was already selected. Starting Codex in the workday walk left the running Codex terminal selected.

Commit `71b0ba9a2` (ov-1470) added the exact-name click and the two-runner CI step. The ov-1485 layout change at `bedde8580` changed the tabs and workspace controls. On that revision, the Web bundles and browser checks job reached tmux inventory recovery on build-box, then timed out after 30 seconds on the link. [GitHub Actions job log](https://github.com/taneliang/farcooler/actions/runs/45911805928/job/140229562752)

## Fix

Lost terminals appear under the live terminals with a visible **Lost** badge. The link keeps the terminal title as its accessible name and exposes the state as its accessible description. A **Recover** button beside the link uses the existing restart action for browsers with Control access. Recovering directly from the sidebar selects the returned terminal and lets Back return to the previous selection. The terminal link can still open the lost terminal's details. Ended terminals remain under the disclosure.

In the narrow desktop sidebar, the first version of the row broke the title into one word per line. Recover now moves below the title when the row is too narrow for both. At 320 px the two stay on one line.

The walk's locator is now scoped to the sidebar, and its recovery click uses the sidebar's Recover button. It has the same timeouts and steps as before.

## Landing gate repair

The first landing gate failed `shared-core-lint.py` (R-112): my Recover button wrote its accessible name, `Recover <title>`, in `work-screen.ts`. The Rust work view now sends the button's label and accessible name. It also sends a `lost` flag, so the page no longer picks lost rows by the display word "Lost". The ratchet baseline is unchanged. The fix is commit `245035172`.

## Verification

The unmodified `bedde8580` bundle reproduced the 30-second timeout at `csp-check.mjs:1320` in local Chromium with two isolated Rust runners. The checkout, Codex launch and tmux health checks had already completed.

Everything below ran at `245035172`, on base `29c41d4de`.

On the Canary bundle, the two-runner workday walk passed three times in a row with no CSP violations. The third run saved the captures, taken before the walk selected the lost terminal. [320 px capture](workday-lost-terminal-320.png) · [1280 px capture](workday-lost-terminal-1280.png)

These also passed: the Work browser suite (including the lost row and one-click Recover at 320 and 1280 px, and a read-only browser with no Recover button), the Board, Conversation and Plan browser checks, the CSP walk with CI's 100 ms tunnel delay, the four-session CSP walk, the split-pane sleep scenarios, the web tests and web workflow tests, both TypeScript checks, the `farcooler-client` library tests (420 passed, 2 ignored) and its Clippy check, and the shared-core lint with its self-test and baseline check. The Canary bundle passed the size gate at 5,097,217 compressed bytes. The stable bundle was not built locally.

The last full `scripts/landing-gate` run passed. Three earlier runs in this session failed:

- Two failed because of how I set up build output (`CARGO_TARGET_DIR`, then a symlinked `target/`). The gate needs a real `target/` directory.
- One failed on a single AgentKit test, `aBoardTickRedrawsOnTheClockAlone`, which waits 5 seconds for a main-actor redraw. It then passed three times alone and in the next full Swift run. This branch changes no Swift. I left the test as it was; it can fail again on a busy Mac.

## CI

**Not yet verified.** No CI run exists for this branch. The CI workflow runs on pull requests and on pushes to branches including `main` and `landing/**`. In an earlier session, the approval system blocked a candidate push because it would publish the code and report to the public origin without explicit authorization. Nothing has been pushed. The failed run ([job log](https://github.com/taneliang/farcooler/actions/runs/45911805928/job/140229562752)) checked `bedde8580`, so rerunning it would test the old code. The acceptance item "passes on CI" stays open until this branch is pushed or landed.
