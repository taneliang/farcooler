# ov-1460 · Clippy before review

Main’s workflow bundle places the Clippy check after quick checks and before
agent review. A failed check returns the candidate and its diagnostic to Build
through the existing rejected route. The builder’s next candidate runs both
checks again.

The check diffs against the engine’s pinned base. When the diff has no Rust
paths, it exits before Cargo or the machine lock. It maps Rust paths to Cargo
workspace members, including shared test modules included from outside a
crate. It takes the default Clippy command and per-package feature variants
from the local landing plan. The default command replaces `--workspace` with
the changed packages. Applicable feature variants keep their full arguments,
including `--all-targets -- -D warnings`. The fixture suite that invokes Cargo
runs in the regular gate, keeping the quick-check stage build-free.

## Regression

The test using Main’s exact check command failed on the old definition because
the stage before review was `quick-checks`. It then passed with the new stage.
The test commits a real Clippy error in the changed crate, first in default
code and then behind a landing feature. Both candidates return nonzero with a
Clippy diagnostic. The repaired candidate passes even though an unchanged
workspace member still contains the same error. The docs-only candidate skips
the check without creating a Cargo target directory.

The quick-check classification test also failed first in both planning modes,
then passed after the compiling fixture suite moved out of the quick phase.

The landing gate, landing policy, gate evidence, engine gate and lane gate
lifetime suites passed 78 tests with a scratch Git home configured for the
`master` default branch. The lane gate self-test and file-size budget passed.
Four targeted Rust tests passed across the daemon, client and store. Main’s
bundle validator found the same stale Canary Codex hook pin in the original
bundle and in this proposal. The validator passed after refreshing that pin
and its derived operation pins in a scratch copy.

## Runner timing

Measured on this Apple Silicon Mac with Clippy 0.1.98, four Cargo jobs and
niceness 10. The daemon-only diff used `8dc6d9795^` as its base and includes
`crates/daemon/src/yielding/tests.rs`. All build artifacts were under
`$FARCOOLER_SCRATCH/target`. The timing excludes the wait for the shared
machine lock and includes process startup and change detection.

| Candidate and cache state | Wall time |
| --- | ---: |
| Daemon, all three Clippy variants warm, three runs | 3.64 s, 3.36 s, 5.44 s |
| No changed Rust files | 0.78 s |

The first pass through all three daemon variants took 386.98 s with default
dependencies already primed and feature variants still cold. All checks passed.
The workflow command uses the shared machine lock and four Cargo jobs.

## Draft filing

The [checked-in bundle](../../.claude/agent/reports/ov-1071/workflow-main.json)
now points to the content-addressed Claude and Codex hook files present on
Canary. Their byte digests match the file pins. All 13 stage and group
operations pin the resulting adapter,
`wf1:e5aa796034a2ccdec6cc62e601bc10855be67db7434130487815900825622205`.
The five Python suites passed all 78 tests again with the refreshed bundle,
including the planted Clippy error and the skip for a docs-only diff.

Canary CLI `0.1.0+5f08afc22` reached Overnight Main on Nov 24
(`01a0e3c7-3e2c-7ed0-b31c-0cc0d7401571`). It returned
`scope denied: known caller required` for `workflow get`, the
configuration-only read used by `workspace re-pin`, and
`workspace workflow-draft-status`. The installed CLI predates ov-1456’s
`workflow bundle` command. The filing verdict is **blocked**. This builder
could not verify the selected configuration against the checked-in bundle,
and obtained no draft ID or Needs Decision card.

An authenticated Main orchestrator or owner session is required to read the
selected bundle and file the change. The filing evidence is the draft ID,
the Needs Decision card ID, the expected published pin, and the draft’s
`pending` state.
