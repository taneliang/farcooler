# ov-1133 Nightly proof

The image gesture correction and its CI diagnosis are documented in
[ov-1097's evidence](../ov-1097/evidence.md).

Nightly run [43360911044](https://github.com/taneliang/farcooler/actions/runs/43360911044)
was dispatched on commit `e12ef7682` after checking that no Nightly was in progress.
See the run for its final status and test results.

Run 43360911044 failed the shard-budget gate on historical main timings;
after recording that failure, it was cancelled to release the runners. The
shards were rebalanced using the measured class times. The corrected branch
was dispatched as [Nightly 43362860903](https://github.com/taneliang/farcooler/actions/runs/43362860903)
on commit `dab98c8dd`, after the queue cleared. See the run for its verdict.

Run 43362860903 passed the shard-budget check but failed the separate CI job
headroom check: a green macOS Rust job took 54 minutes against a 60-minute
limit. It was cancelled after that failure, and the limit was raised to 70
minutes. [Nightly 43363154074](https://github.com/taneliang/farcooler/actions/runs/43363154074)
was dispatched on commit `5d471f3c1` after the queue cleared. See the run for
its verdict.

Run 43363154074 passed the timing gates but failed the `phone3` workspace
accessibility audit (Dynamic Type findings on three subagent-row texts) and
`captures3` Plan captures (quiet-page timeout and page-three scroll state).
The corresponding three tests passed locally on iOS 27.0; CI uses 27.2. The
run was cancelled after those failures. The notice fixture was also corrected
and its focused test passed locally; the preceding ov-1097 validation had
identified the missing field as a separate failure in the image test class.

Nightly [43369485423](https://github.com/taneliang/farcooler/actions/runs/43369485423)
was already running on commit `97f483692` when this investigation resumed.
Its Mac perf gates job failed: the scrolling test measured a median main-thread
hitch of 75.005797 ms/s against the 75.0 ms/s CI budget. This branch does not
change the scrolling implementation; the 0.005797 ms/s overage on a shared
runner warrants a retry rather than an unverified change to the budget. GitHub
rejected a job retry while the workflow is still running. At the last check,
the phone6 image-test shard remained queued. The Nightly has not passed, and
neither the image test's CI result nor the requested green proof is established.

Nightly [43374015223](https://github.com/taneliang/farcooler/actions/runs/43374015223)
was dispatched on `1273bc3eb` after run 43369485423 was cancelled and the
queue cleared. Its Mac scrolling check passed, but the first-page performance
check measured a 164.7295 ms stall against the 100 ms CI budget; the following
streaming test measured its own first page at 54 ms. The `captures3` Plan
capture again failed to reach the page-three scroll state on iOS 27.2, and
`phone2` failed two notification-tap tests because the harness never marked
a notification scheduled. The run was still in progress when these findings
were recorded; `phone6` had not started. These failures do not establish a
green Nightly or an image-test result.
