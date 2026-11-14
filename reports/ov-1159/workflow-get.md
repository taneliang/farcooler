# ov-1159: workflow reads under load

The investigation source requested by the ticket is committed at
`reports/ov-1157/landing-rca.md` without changes.

## Cause and change

A CLI read used the same bounded inbox as the single engine writer. The writer
polls effects and does per-message fold-tree cleanup before serving another
message (`workflow_runtime.rs`, `sweep_loop`). A read could therefore wait for
unrelated work. The old reply also repeats each item's definition and includes
notification delivery history and finished groups.

The CLI now opts into a summary with the existing request envelope's `accepts`
field (`workflow_summary_v1`). `--full`, and the CLI's internal draft/template
reads, select `workflow_full_v1`. Older app requests retain their full schema;
an older runner ignores the marker and returns its original full answer.
Neither marker conveys authority.

These reads bypass the effect inbox. The command handle retains the same
committed store and receipt directory; SQLite reads, stop-receipt file reads,
and JSON construction run in `spawn_blocking`. No reducer inputs, git commands,
cleanup, or provider operations occur on that read path. Admission and runtime
liveness are checked before and after the worker, and full adapter environments
remain hidden from nonowners. Mutations retain the single writer and its final
inbox authority checks.

The summary keeps state revision, epoch, source sequences, and item run, pin,
phase, numeric stage, members, grouped_in, candidate and tree. Empty membership
and absent grouping/tree have explicit array/null values. Item definitions and
configuration definitions/adapters, notification history, deduplication IDs,
and finished groups are omitted. Ticket archives remain available; group
archives and finished group items are hidden only in the projection. `--full`
returns the original definitions and history. Nothing is deleted from storage.

CLI reads have a ten-second deadline with a specific timeout sentence. Mutation
requests have no new deadline or automatic retry, so an uncertain write is not
silently resubmitted.

## Verification

Builds use this worktree's `target/`, with `TMPDIR=/tmp` for compiler temporaries
and short private test socket paths. No build target was placed under `/tmp`.

The new CLI parser test failed on the original code because `--full` was not
accepted. The paused-clock deadline test failed because the read remained
pending past eleven seconds. The summary projection test failed on the original
identity projection because the finished group was still present. The held
inbox test leaves the engine inbox completely unconsumed and requires a real
RPC reply within two seconds, comparing the required caller fields and the
full state against the original wire reply.

The held-inbox test was observed failing on the old daemon at its two-second
RPC deadline. The opt-in load regression also failed against the old daemon:
a default CLI read still contained `/state/items/load-0/definition`. All five
new tests were therefore seen red before their fixes. The paused-clock/parser
checks and all three daemon read/redaction checks pass with the fix.

## Measured load result

Final measurement on 2026-11-14, debug build with the repository's optimized
dependencies, private local runner, 15 concurrent build stages. Each stage
runs a real Python adapter process hashing 1 MiB buffers continuously; each
writes its unique run/PID marker before entering the CPU loop. The benchmark
waits for all 15 markers and asserts on every reply that all 15 items remain
Running at build stage 1. These are controlled CPU-active build adapters,
not signed-in provider agents or fifteen Cargo compilations. The installed
Canary runner and its workflows were not modified.

The measured operation is a fresh `target/debug/farcooler workflow get <ws>
--json` process through the real Unix socket and authenticated RPC dispatcher,
including process startup, connection, server work, JSON output and exit.
Forty sequential samples, nearest-rank p95 (sorted sample 38):

| Metric | Result |
| --- | ---: |
| Concurrent active builds | 15 |
| Samples | 40 |
| Lower median (20th sample) | 0.0117 s |
| p95 | **0.0150 s** |
| Maximum | 0.0288 s |
| Summary reply | 14,881 bytes |
| `--full` reply for the same live state | 38,263 bytes |

The same test invokes `--full --json` and asserts that the definition's stages
are present, before timing the summary. An earlier successful 40-sample run
measured p95 0.0146 s and maximum 0.0499 s. Both are below the two-second target.

Reproduce after `cargo build -p farcooler-cli`, using this worktree's target:

```sh
TMPDIR=/tmp cargo test -p farcooler-daemon --lib \
  workflow_get_cli_p95_with_fifteen_builds -- --ignored --nocapture
```

The test needs permission to start its private git/process/socket fixtures.
Initial sandboxed fixture attempts failed before exercising RPC; the same
compiled checks passed with access to those fixtures. Compiler temporary files
also required explicit `TMPDIR=/tmp` in this environment.

The workflow read call-path source check confirms `Exec::read_get` contains no
process launch, git/status call, effect pump or tree pruning. The RPC marker
branch calls `CommandHandle::read_get`, not `wire_command`; the held-inbox test
proves this independently of the source check. Existing removal/mutation
operations keep their required safety checks; the workflow read does not run
them or their per-message cleanup.

`--full` retains the existing transport frame limit and complete history.
The summary view is the smaller default for inspecting active work; full
inspection remains an explicit choice.

## Final checks

| Check | Result |
| --- | --- |
| `cargo test -p farcooler-cli --bin farcooler -- --test-threads=6` | 633 passed, 1 opt-in test ignored |
| `cargo test -p farcooler-daemon --lib workflow_ -- --test-threads=6` | 348 passed, 5 opt-in tests ignored |
| `cargo test -p farcooler-daemon --test workflow_commands_over_socket --test workflow_service_over_socket -- --test-threads=4` | 2 command/socket tests and 18 service tests passed; 1 subprocess helper ignored as a standalone test |
| Compiled daemon `workflow_get_ --include-ignored --nocapture` | 4 passed, including the final load run and full CLI definition check |
| `cargo clippy -p farcooler-cli -p farcooler-daemon --lib --bin farcooler -- -D warnings` | Passed |
| File-size budget, swallow lint, staged doc-comment check, staged diff whitespace check | Passed |
| Protocol lint | Capability table passed; this repository has no preview baseline to compare |

The affected Rust suites ran with `TMPDIR=/tmp` and access to their private
process/socket fixtures. The ordinary suite's opt-in load test was explicitly
run in the separate measurement above. No reducer, `coding_*.rs`, schema or
protocol definition was changed. The only runtime-structure change is at the
command handle's RPC seam, retaining read access beside its mutation sender.

Acceptance: p95 is below 2 s with 15 active builds; required caller fields and
`--full` definitions are verified; workflow summary/full CLI reads never enter
the actor's git/status or cleanup path. All requested implementation and test
work is complete.

## Fix round 1: dispatch snapshots keep definitions

The review found one internal read still using the summary marker:
`plan_agent_start::snapshot`. Dispatch identifies a tracked ticket by
`items[k].definition.source` and reads model choices from
`configuration.definition.models`. The summary removes both, so a waiting
ticket was classified as already running and model selection was lost.

Dispatch snapshots now use an explicit full request, shared with `--full` and
the internal draft/template reader. The public default stays a summary. The
other internal request caller, `tasks::runs_a_workflow`, only tests whether
configuration exists, which the summary preserves.

The new CLI regression uses the real Service and RPC handler, a published
configuration with two model choices, and an actual tracked ticket. It first
checks the default summary omits definitions, then calls `snapshot` and checks
that the WaitingApproval ticket is AtRest (with the same run and pin), both
models remain available in order, and selecting a configured model succeeds.
Before the fix, this test failed with `Taken("already has a run under the
engine")` instead of `AtRest`. With the fix it passes.

Round 1 validation: the full CLI suite passed (634 tests, 1 opt-in test
ignored), and CLI clippy including tests passed with warnings denied. The
suite needs access to private socket/process fixtures; a sandboxed run failed
on permission errors before the permitted run passed. File-size budget,
swallow lint, doc-comment check and diff whitespace checks passed. All builds
used this worktree's `target/` with `TMPDIR=/tmp`.

The three daemon workflow-read regressions passed again. The combined Cargo
run's opt-in load fixture timed out before all build-start markers appeared;
no latency samples were taken in that attempt. Running the same compiled
fixture alone from `crates/daemon` passed with the rebuilt CLI: 15 active
builds, 40 samples, median 0.0130 s, **p95 0.0139 s**, maximum 0.0168 s,
summary 14,881 bytes, full reply 38,263 bytes. The fixture also verified
`--full` definitions and all fifteen Running/build-stage items in every
sample. The startup failure in the combined attempt remains unexplained;
the isolated rerun reproduces the original acceptance result.

## Fix round 2: all workflow readers bypass cleanup; RPC status scans use workers

The second review identified an unmarked app Get still entering the effect
inbox, and worktree removal performing its safety scan on the RPC executor.
Every Get now uses the committed read path. No marker means the original full
schema, including definitions and notification history; the CLI summary marker
still selects the small view. Admission rechecks and nonowner redaction remain
in the shared reader. The dispatch full-view fix from round 1 is retained.

The removal confirmation scan now runs on a blocking worker. The RPC waits for
its fresh answer before accepting a typed name or removing anything; a worker
failure refuses the operation. The original service check still includes dirty
files, hidden hooks and edited manager skills. The RPC regression verifies
that absent and incorrect names preserve dirty files, and that a clean worktree
can still be removed without a name. Existing service safety checks also pass.

The broader status call-path audit also found four Changes handlers whose
review cache can miss (`change_set`, `file_diff`, `set_base`, `mark_read`), and
worktree creation/adoption rollback checks. Those operations use the same
worker seam. Their futures are polled on the blocking worker using the daemon
runtime's I/O driver, so filesystem inspection and status-result processing
stay off the RPC executor. Request/result schemas and service safety rules
are unchanged. Workflow mutations still use the single-writer inbox.

Before the fixes, the stalled-inbox regression failed at its two-second
deadline specifically for an unmarked Get. The removal source guard failed
on the direct service scan, and the expanded Changes source guard failed on
`changes.change_set`. All three pass after the fixes. The guards now cover
removal, creation and the four Changes handlers, plus the actual stalled-inbox
RPC test covers summary, explicit full and unmarked full reads.

Round 2 load measurement, using the same isolated CPU-active adapter fixture
and this worktree's existing CLI: 15 Running build-stage items throughout,
40 samples, median **0.0113 s**, **p95 0.0147 s**, maximum **0.0157 s**.
Summary 14,881 bytes; full reply 38,263 bytes, with definitions verified.
The rebuilt daemon test executable ran alone from `crates/daemon` with
`TMPDIR=/tmp`; build artifacts stayed in the worktree's `target/`.

Round 2 validation: workflow library suite 348 passed / 5 ignored; workflow
socket suites 20 passed / 1 helper ignored; RPC scope and regression suite
13 passed; removal-filter suite 16 passed; existing removal service suite
6 passed. These fixture runs used access to private git/tmux/socket processes.

The review-filter library suite passed all 104 tests; the socket integration
for worktree creation and asynchronous PR fills passed both tests. Final
compiled workflow-read regressions passed all three ordinary checks, and the
13 RPC scope/source/safety checks passed again after the worker expansion.
`cargo clippy -p farcooler-daemon --lib --tests -- -D warnings` passed. The
existing load fixture's needless borrowed socket path was corrected to satisfy
that test-target lint. File-size budget, swallow lint, staged doc-comment and
whitespace checks passed. Protocol lint checked the capability table and
reported that no preview baseline exists. No reducer, coding module, protocol
schema or service implementation was edited.
