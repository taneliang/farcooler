# ov-1523: the sleep check passes on the new shell

## Cause

**`8c996be15`** ("ov-1490: Web: Add Runner, runners not set up here, and approving or removing devices") changed the first screen of a new browser. It reached `main` in the push ending at `33b4c71ef`. It's not in `669c23876`, the last commit where CI was green.

- **Before:** after Make Passkey, a new browser opened on `#enroll`.
- **After:** `firstScreen()` (`services/web/src/client/enrolling.ts`) opens Add Runner (`#runner-add`). `#enroll` is behind **Approve From Another Device**.

`sleep-check.mjs` still waited for `#enroll` right after Make Passkey, so it timed out after 90 s, before any sleep case ran. Locally, the page was polling `ceremony/reply`, as a waiting screen does, while the check waited.

**The check was wrong, not the page.** The new first screen is intended: ov-1490 documents it, and the same ticket updated `csp-check.mjs` to follow it. It didn't update `sleep-check.mjs`.

## Fix

The check now waits for Add Runner, selects Approve From Another Device, then does the same `#enroll` wait as before. This also makes it fail if a new browser doesn't open on Add Runner. No case, assertion or existing timeout changed. The new wait uses the same 90 s limit as its neighbors. No product code changed.

`sleep-check.mjs` failed locally with CI's error before the fix. After it, all 8 cases pass, `busy-runner-5s` among them (reconnected without help, input reached the runner from both panes). The three dead-link cases showed the notice after about 14.2 s, within the 17 s limit.

## Browser checks in ci.yml's web job

The `Web` job only reports the results of `Web tests` and `Web bundles and browser checks`. Its failure on `33b4c71ef` was `Web tests: success, Web bundles: failure`. The browser checks are all in the bundles job.

Each ran locally on this branch at `d694771af`, the fix commit, in CI's order, and passed:

| Check | Result |
| --- | --- |
| `chat-check.mjs` | passed |
| `chat-files-check.mjs` | passed |
| `plan-render-check.mjs` | passed |
| `shell-check.mjs` | passed |
| `work-check.mjs` | passed |
| `board-check.mjs` | passed |
| `csp-check.mjs --fake-daemon … --tunnel-delay 100` | passed, no CSP violations |
| `sleep-check.mjs --fake-daemon … --engines chromium --split` | passed, 8 of 8 cases |
| `csp-check.mjs --join` (two real runner binaries) | passed, no CSP violations |
| `csp-check.mjs` workday walk | passed, no CSP violations |

The job's other steps passed too: the size gate's self-test, the stable and canary bundles, both size gates, both `wrangler deploy --dry-run` runs, and the builds of the check binaries.

This was on macOS with Node 24; CI runs Linux. For the two walks, `TMPDIR` was the step's scratch directory.

The ov-1491 report says the workday walk stopped because the Plan never showed Checking Here. It passed here. I didn't look into why the results differ.

## CI

- Run 46085159298, on `571a7334b` (`main` before this fix), failed at the same sleep step.
- **Not yet confirmed:** green `Web` and `Web bundles and browser checks` jobs on `main`. That needs this branch to land; I didn't push.
- Because CI stops at the sleep step, the workday and join walk step after it hasn't run on Linux in the failing runs. It passes locally, but its first CI result on this code will come after landing.
