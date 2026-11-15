# One flaky test does not fail a landing (ov-1285)

The local lane gate and the landing gate run the Rust suite through `scripts/gate-flaky.py`, with nextest's
`gate` profile. CI on main is unchanged: profile `ci`, no reruns, so a real flake still shows there.

- After the main run, each test that failed or timed out is rerun once, serially, with nothing else running
  (`--test-threads 1`, a filter naming that one test). The gate passes if every rerun passes. A test that
  fails its rerun fails the gate exactly as before.
- Each test that needed a rerun is printed as `FLAKY: <binary> <test>: first 1.2s (failed), rerun 1.0s (passed)`.
  The line lands in the step log, in the lane-gate summary (a `### FLAKY tests` section), in the v2 gate
  evidence (the receipt's `flaky` list and the sealed `summary.txt`), and in `.config/flaky-log.tsv`, the
  tracked log the orchestrator files one ticket per line from.
- The gate profile stops a hung test after 120 s (60 s period, terminate after 2) instead of 600 s. A test
  known to be slow keeps an explicit `[[profile.gate.overrides]]` in `.config/nextest.toml`; the live tmux
  stress tests in `live_tmux` have one (10 minutes).

## The three tests that failed the landings of Nov 16

All in `farcooler-daemon`, `workflow_runtime`, each failing one landing and passing normally:

| Test | Normal | Failed run |
| --- | --- | --- |
| `script_hooks_tests::script_hooks_good_bad_and_illegal_decisions_are_logged_and_replayed` | about 1 s | FAIL at 1.2 s |
| `gate_tests::cancelling_parallel_review_ends_the_owned_gate_before_returning` | 2.1 s | FAIL at 5.4 s |
| `concurrency_tests::workflow_get_shows_an_effect_waiting_for_a_place` | 1.0 s | TIMEOUT at 600 s (120 s now) |

Normal durations are from passing runs in the group-2895 gate outputs; the first test has no passing line
there, so its normal time is the ticket's "about a second".

Tests: `python3 scripts/gate-flaky.py --self-test` (a fixture that fails once then passes, and one that
fails twice), and the same two cases through a real gate step in `scripts/lane-gate.py --self-test`.
