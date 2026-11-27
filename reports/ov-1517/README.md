# ov-1517: the script hooks restart test

## The race

The rule's deadline raced the setup of the rule's own checkout.

- The test fixture gave every rule `timeout_ms: 200`. The deadline is set when the rule fires and is measured in real time while the call runs.
- Before the script starts, `execute_hook` writes a private copy of it with an fsync, reads the state and runs `git worktree add`. The script gets whatever time is left, and it still has to start `sh` and `sed`.
- With temporary timing prints, about 190 ms was left when the call started and 115–157 ms after the checkout, over the calls I sampled (1-minute load average up to about 15). The unchanged test also failed on my first local run, before I added any load.
- On a busy Mac the rest runs out. The call returns `Timeout`, and the assertion that expects a `Receipt` fails.

The product code behaves as written. Only the fixture changed. Rules that a test expects to answer now get 60 s. The `/bin/sleep 2` case and the fake-clock expiry test keep 200 ms. No assertion was removed or weakened. The failing assertion now prints the outcome it got.

The same 200 ms applied to the other tests in this file that expect an answer, including the `exit 1` case, which expects `Missing` and could have got `Timeout`. `docs/ci/gate-flaky.md` lists `script_hooks_good_bad_and_illegal_decisions_are_logged_and_replayed` as failing a landing on Nov 16. This race could explain that, but I haven't seen that log.

## Runs under load

Load was 10 `yes` processes on a 10-core Mac plus one process rewriting and fsyncing a 4 MB file, alongside whatever else the Mac was running. Each run started the built test binary again.

| Code | Filter | Runs | Failed | Load average, start to end (1 min) |
| --- | --- | --- | --- | --- |
| Before the fix | `script_hooks_restart` | 200 | 31, all `Hook(Timeout)` | 12 to 39 |
| After the fix | `script_hooks_restart` | **200** | **0** | 22 to 33 |
| After the fix | `script_hooks_` (the whole file and any other match) | 60 | 0 | 31 to 42 |

## Why the landing failed although the test was FLAKY

**The FLAKY result didn't fail it. The leak check did.**

The Rust step is `test-leak-check.py -- tmux-leak-check.py -- gate-flaky.py run -- …`. The rerun passed, so `gate-flaky.py` had nothing to fail on. After it printed the FLAKY line, `test-leak-check.py` found `.tmpkryjQj` in its private `TMPDIR` and exited 1. The log (`rust-40566172.log`) ends in that order. The summary's Result column reads `FAIL (exit N); FLAKY x1 (rerun passed, listed below)` for such a step, which names the flake and not the leak check. I found a saved row with that wording.

Both rules stay as they are. The reason is now written where the builder sees it:

- `test-leak-check.py` ends a run whose command passed with `FAILED on leftovers alone: the command passed (exit 0). If a test above is marked FLAKY, its rerun passed and it did not fail this run.`
- The gate summary's Result column adds `failed by test-leak-check alone: …; a FLAKY rerun did not fail this`.
- `docs/ci/gate-flaky.md` says the same.

Both self-tests cover this. I watched the new `lane-gate.py` check fail before `test-leak-check.py` printed the line.

## Not found: what left `.tmpkryjQj`

- This test makes its folder directly in `/tmp`, outside the leak check's private `TMPDIR`.
- I forced its timeout 20 times with a private `TMPDIR` (a 15 ms deadline). Nothing was left. That doesn't cover every moment at which a call can time out.
- The leak check deletes the leftover when it exits, so what was in it is lost.
- In the saved Rust logs I searched, 11 printed a FLAKY line and 2 of those also left a `.tmp*` entry: this run, and one where `workflow_service_admitted_ci_recovers_original_bundle_after_reconfiguration` flaked. A third run left a `.tmp*` entry and isn't one of those two. This suggests failing or loaded runs can leave a folder behind, but doesn't show which test did.

So a landing can still fail this way. A follow-up could have the leak check list what is inside a leftover folder, so the next one can be traced.

## A product question this raises

A real rule's `timeout_ms` also includes the `git worktree add`. In a large repository, a short timeout could expire before the script starts. I didn't change this.
