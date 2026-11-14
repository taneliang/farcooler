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
partition mutation tests and a real five-case nextest archive (four runnable,
one ignored), Mac selection guard/self-tests and leak-wrapper self-tests.

A temporary branch-only push workflow copies the production Rust jobs, removing
only the changes/path-selection dependency. It will be removed after timing.
