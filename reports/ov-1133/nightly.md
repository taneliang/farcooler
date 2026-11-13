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
