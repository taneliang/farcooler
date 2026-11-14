# ov-1182: Rust CI under 15 minutes

Baseline main run: [43557233452](https://github.com/taneliang/farcooler/actions/runs/43557233452).
Linux Rust: 24m10s; macOS Rust: 39m01s. Full tests plus shadow bookkeeping:
15m35s Linux, 26m04s macOS. Raw step timestamps are in baseline-jobs.json.

Implementation reuses the ov-1181 Mac selection and its maintained coverage
guard. Full Mac tests remain in Nightly. Linux compiles default-feature tests
once into a nextest archive and runs four hash partitions on independent
runners. Each runner lists the full archive and every partition, rejecting any
missing, unexpected, changed or duplicated runnable test before execution.
Ignored tests keep their existing policy. Feature tests, clippy checks,
target checks, doctests and both leak wrappers remain per push.

The archive explicitly carries the runtime CLI, daemon and LFS filter. Consumers
extract under their own worktree target, recreate channel CLI aliases and set
the same test shell. Shadow measurement moves to Nightly; it is an experiment
with inventories, host-load waits and a possible sequential selected replay,
not a required coverage check. Hosted before/after evidence follows below.

Local validation: actionlint for changed workflows, ci-changes self-test,
partition mutation tests and a real 65-case nextest archive (64 runnable,
one ignored; all four partitions executed successfully), Mac selection guard/self-tests and leak-wrapper self-tests.

A temporary branch-only push workflow copies the production Rust jobs, removing
only the changes/path-selection dependency. It will be removed after timing.

## Shadow measurement limits

The baseline Linux full command took 588.47 s (584.716 s in nextest execution)
inside a 935 s step. Mac took 1040.50 s (1034.879 s execution) inside a 1564 s
step. Neither replayed a narrowed proposal on this workflow-changing commit.
The remaining 346.53/523.50 s include test compilation during inventory and
planner/load/bookkeeping time; they cannot all be attributed to shadow overhead.
Moving shadow removes optional sequential replays from per-push critical paths;
the baseline alone does not prove minutes saved by removing its bookkeeping.

## Initial experiment and follow-up

Run 43573567048 measured the first split. By roughly 12 minutes the Mac job
was still in feature/check setup, before its default suite. The final design
therefore runs native Mac tests independently of the unchanged platform
lint/feature checks. The initial harness is initial-hosted-timing.yml and is
not counted toward acceptance for the final job layout.

Rust CI sets CARGO_PROFILE_DEV_DEBUG=0 and CARGO_PROFILE_TEST_DEBUG=0 to reduce
binary/link/archive costs. These settings change debug symbols, leaving Cargo's
optimization settings and debug assertions unchanged. Artifact binaries retain
panic messages and test names but do not carry full debug symbols. The symbol settings leave source,
test selection, retry policy, required verdicts and test timeouts unchanged.

The inherited ov-1181 repository-wide actionlint annotations are included so
plain `actionlint` passes, including the custom xcode-27 runner label.

The initial Linux checks passed in 916 s (15m16s), and the original debug-symbol
archive job passed in 921 s (15m21s). The monolithic Mac job exceeded 20 minutes
before completion. That superseded experiment was cancelled to free hosted
runners; cancelled shards/Mac have no passing verdict and are not acceptance
evidence. Its metadata is initial-jobs.json.

Initial archive payload: 6,313,939,230 bytes; compression took 100.47 s and
upload took about 70 s. Two shards uploaded partial inventories during
cancellation; no guard completion or full execution verdict is counted. Disabling debug
symbols specifically addresses this observed archive cost.
