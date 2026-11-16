# ov-1312: resize scroll test synchronization

The failure is in [CI run 43962297903, attempt 1, job 133632778096](https://github.com/taneliang/farcooler/actions/runs/43962297903/job/133632778096), on `91c9c5bb57bb7e227826ef48133e081a0d6f8f95`. That commit changes only `reports/ov-1310/README.md`. The job log reports `now == top` at `TranscriptScrollerTests.swift:845`, with the reader moving from `real:198` to `real:196`, after 33.290 seconds. It reports no knob assertion failure. The Mac tests ran in two processes; their recorded five-minute load peaked at 17.8374.

The test's resize steps waited for three unchanged geometry polls, each separated by a nominal 20 ms. That condition does not mean the resize is finished: `TranscriptScroller` restores the saved reading point after `widthFrames` (15) display-link ticks, then may aim for a row that needs to be built. The next test resize could arrive before or after that restoration, depending on layout cost and scheduling. A late step can release the old hold and capture another row on the next step. This is the synchronization race identified from the code; the CI log records its result, but does not contain intermediate ticker state.

A temporary diagnostic run of the original test confirmed the condition mismatch locally: all 69 width steps returned a successful geometry poll while both `ticking` and `restorePending` were true. The test passed in that run (15.069 seconds), which also shows that a successful geometry poll alone cannot distinguish a pending restore from a completed resize. The diagnostic prints were removed.

Each resize step now waits for the existing `laidOut(done: true)` condition: stable content height and clip origin, no active ticker, and no pending row restoration. Initial and final waits, as well as every width step, must succeed; a timeout fails at the wait instead of allowing measurements from unfinished state. The row identity, 2% knob displacement, knob agreement, row membership, and 4% knob-size assertions are unchanged. No application behavior changes.

The comparison used the same Mac, Xcode 27.1, ARM Python, `TMPDIR=/tmp`, foreground execution, and four build jobs for both versions:

```sh
PATH=/Users/e-liang/.local/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin:/Users/e-liang/.cargo/bin \
TMPDIR=/tmp swift test --package-path apps/macos --jobs 4 \
  --filter aResizeHoldsTheReader --maximum-repetitions 20 --repeat-until fail
```

The original test passed all 20 repetitions in 308.787 seconds. Its CI failure was not reproduced: this filtered local run did not recreate the two-process CI workload or its exact display/layout scheduling. This is a limitation of the comparison, not evidence that the original synchronization was safe. The separate diagnostic run above demonstrates that its wait returned before restoration completed.

The fixed test passed **20 consecutive repetitions** in 350.878 seconds, exit 0. Both comparison logs show repetitions 1 through 20 (the first start has no repetition suffix), followed by the passing summary. See `with-fix.log`, `without-fix.log`, `baseline-diagnostic.log`, and `ci-excerpt.log` beside this report. The first standalone fixed run also passed in 17.056 seconds.

`apps/macos/test.sh --jobs 4` passed, exit 0, with the same PATH and TMPDIR plus `CARGO_BUILD_JOBS=4`. The main bundle ran 1,979 tests in 352 suites in 140.832 seconds (floor 1,950); `TranscriptScrollerTests` passed in 99.922 seconds, including the changed test. All other test bundles passed, and the temp-file/preferences and tmux leak guards passed. See `macos-test-summary.log`.

The runs used foreground commands, at most four build jobs, and no full gate, formatter, or unrelated code changes. The initial sandboxed attempt could not create compiler temporary files/cache entries; the native build and tests were rerun with the required cache/window-server access. The system PATH also selected an incompatible Intel Python for script shebangs, so the successful commands select the installed ARM Python explicitly. Neither issue required repository changes.
