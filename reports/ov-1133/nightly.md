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
