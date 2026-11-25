# ov-1460 · Clippy before review

Main's workflow bundle runs changed-crate Clippy between quick checks and
agent review. A nonzero exit uses the engine's existing rejected route to
return the candidate and diagnostic to Build. The builder's next candidate
runs both checks again. [Workflow bundle](workflow-main.json)

The gate diffs against the engine's pinned base. It exits before Cargo or the
machine lock when the diff has no Rust paths. It maps changed Rust paths to
workspace members, including shared test modules outside a crate that a
workspace member includes. It takes Clippy flags from the landing plan. The
workspace command replaces `--workspace` with the changed packages and each
applicable feature variant keeps its full arguments, including
`--all-targets -- -D warnings`. The compiling regression suite runs in the
regular wire gate so quick checks remain build-free. Wire CI provisions
Clippy for the fixture.

## Regression

The workflow regression first failed against the old definition because review
was preceded by `quick-checks`. On the rebased branch it also failed in a
scratch copy with Clippy execution disabled. The disabled check returned zero
for the candidate with the planted lint.

The workflow command rejects the planted `clippy::manual_filter` lint in both
default code and code enabled by a landing feature. It accepts the repaired
candidate while an unchanged workspace member still contains the lint. A
docs-only candidate skips Clippy without creating a Cargo target directory.
The quick-check classification test also went red in both planning modes
before the compiling fixture suite moved to the regular gate.

On the rebased branch, 90 tests passed across the landing gate, landing policy,
gate evidence, engine gate, lane gate lifetime, and Rust selection suites.
The CI change-selection and lane gate self-tests passed, as did the file size
budget. The tests used a scratch Git config with `init.defaultBranch = master`,
four Cargo jobs, and niceness 10.

The earlier daemon bundle validator passed after the hook and operation pins
were refreshed. On the rebased branch, all 13 operation pins and both
content-addressed hook files matched, and the bundle's configured base was an
ancestor of its landing ref. The fresh daemon validator run was stopped while
the shared runner was above its load limit, before Cargo started.

## Runner timing

Measured on this Apple Silicon Mac with Clippy 0.1.98, four Cargo jobs and
niceness 10. The daemon-only diff used `8dc6d9795^` as its base and includes
`crates/daemon/src/yielding/tests.rs`. Build artifacts went under
`$FARCOOLER_SCRATCH/target`. Wall time includes process startup and change
detection and excludes time in the shared machine lock queue.

| Candidate and cache state | Wall time |
| --- | ---: |
| Daemon, all three Clippy variants warm | 3.64 s, 3.36 s, 5.44 s |
| No changed Rust files | 0.78 s |
| Rebased ticket, exact workflow command, no changed Rust files | 0.24 s |

The first pass through all three daemon variants took 386.98 seconds with
default dependencies already primed and feature variants still cold. All
checks passed. The workflow uses the shared machine lock and four Cargo jobs.

## Orchestrator handoff

The complete bundle pins the content-addressed Canary Claude and Codex hook
files and uses the adapter pin
`wf1:e5aa796034a2ccdec6cc62e601bc10855be67db7434130487815900825622205`
in all 13 stage and group operations.

After landing, the admitted Main orchestrator can export Main with
`farcooler workflow bundle 01a0e3c7-3e2c-7ed0-b31c-0cc0d7401571 --json`
using an app that includes ov-1456. Compare the selected bundle to this
proposal, carry forward any live changes, and use
`workspace workflow-draft-status` to check for pending owner decisions before
filing `reports/ov-1460/workflow-main.json` for owner approval. The builder
handoff is the checked-in bundle.

Earlier builder reads of Main returned `scope denied: known caller required`
on Canary `0.1.0+5f08afc22`, which also predates `workflow bundle`. The live
selection still needs the orchestrator's comparison.
