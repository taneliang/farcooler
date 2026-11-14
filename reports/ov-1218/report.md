# ov-1218 — landing gate evidence

The ov-1185 branch was rebased onto the run's pinned main base
`73974f89898d716c36595eade2511ba5f8df7a8f`. Its changes and prior evidence
remain in this branch. The lane-gate and heavy-lock self-tests passed after
the rebase, using native Python 3.14 and unsigned temporary fixture commits.

The three-run median of real landing gates is **unmeasured**. It will be
measured after landing from real landing gates in the ov-1215 report; file a
follow-up if it misses 15 minutes. The earlier scheduling replay is not a
measurement of that median.

## Checks and scheduling

The local landing gate runs families concurrently, with Rust Clippy separated
from Rust tests and Swift, Swift tests, and iOS serialized together. It retains
the heavy queue and load admission. Release app assembly and its dependent
bundle checks run in CI instead of the local landing gate; the source
entitlement check remains local. The default-host Rust nextest command can
select the affected reverse dependency closure against a verified green base,
and falls back to the full workspace when evidence is unavailable; full CI
remains unchanged. The shadow instrumentation's duplicate full and selected
runs are replaced locally by one nextest run. These are changes in where or how
proof runs, not evidence that every full CI check executes locally. Other
feature checks, lints, doctests and platform suites remain as described in
[the ov-1185 report](../ov-1185/report.md).

## Completed gate and baseline

One real landing wrapper completed green in 1,524.62 seconds. Its
[raw log](../ov-1185/fix-round-1/earlier-green/trial-1/gate.log),
[metadata](../ov-1185/fix-round-1/earlier-green/benchmark.json), and
[per-step seconds beside the historical capture](../ov-1185/fix-round-1/step-timings.md)
are retained. The Nov 14 baseline family totals were Rust 778 s, Swift tests
563 s, Android 536 s, Swift 247 s, and wire 225 s. The linked table gives
individual captured step seconds for both the baseline and the completed gate;
steps absent from either capture are identified there. The historical and new
inventories differ, so their totals are not an apples-to-apples speed ratio.

This completed gate used an earlier dirty snapshot at `1dcb25bd7`, before the
final index and interruption fixes and this rebase; it is not a fresh run on
the final code. A new timing gate was not started: the machine-wide heavy lock
was occupied by an iOS run and another gate was queued. The queue was observed
without joining or bypassing it, and the wait was stopped well within the
30-minute limit. Consequently there is no completed final-code timing here.

Additional verification after rebase: Rust shadow tests and gate summary
self-test passed, as did shell syntax and diff whitespace checks. The initial
sandboxed self-tests could not use native process visibility or sign scratch
commits; the successful runs used native process visibility and disabled
signing only for temporary fixture commits.
