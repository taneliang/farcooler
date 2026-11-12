# ov-1105: Landing queue groups

The plan's Landing section renders the engine's open groups on Mac, iPhone,
Android and web. Rust supplies their order, membership, step and phase, elapsed
step time, split explanation, failed command/job and first failure line. Clients
only lay out the supplied lines. Mac and iPhone share the same SwiftUI group view.

The apps' shared session read includes the authenticated workflow presentation.
Read-only and older connections keep their existing plan when workflow inspection
is explicitly refused. Command gates retain bounded diagnostics through a split,
including both halves; the standard CI provider retains a failed job and its first
failed-log line. Existing records/providers without that evidence remain readable;
this change does not invent historical log lines or replace pinned provider settings.

## Verification

- Entire workflow suite: 144 tests passed, including state serialization compatibility,
  split halves retaining diagnostics, and deterministic replay.
- Client suite: 350 passed, one pre-existing ignored test. Covers the shared queue
  golden, clock skew, absent workflow, split presentation, and read-only/older plan reads.
- Workflow runtime: 270 passed, three pre-existing ignored tests.
- The real command-gate diagnostics checks also passed with setup output followed
  by two errors: the retained line is the first error, and the job names the gate.
- CI starter/provider: four tests passed, including failed-job/log-line retention.
  The provider's shell syntax and jq expressions were also checked locally.
- AgentKit: 2,110 tests passed.
- Mac PlanSectionViewTests: 15 passed, including a 320 px long-title layout test.
  That test draws the exact shared group view used on iPhone.
- Android unit suite passed with the fresh host JNI library. The actual Compose
  Landings section also passed its 320 dp render check with long titles and failures,
  with light/dark captures generated under /tmp/ov1105-android-captures.
- Web: 287 tests passed; client/server TypeScript and wasm target checks passed. The production
  renderer showed all four golden groups at 320 px with no horizontal overflow.
- Generated wire types, file-size budget, copy, swallowed-call, visual-token and
  doc-comment checks passed. iPhone renderer syntax parsed successfully. The
  repository iPhone Plan and smoke command was attempted, but could not acquire the
  simulator leased by another run; no iOS UI tests executed.

The queue golden was demonstrated red with the snapshot integration removed at
its call site, then green after restoration. Native and runtime tests were rerun
with the local file/socket/process access their setup requires after sandbox-only
setup failures. No application deployment or workflow publication was performed.

## Follow-up verification

The CI provider now reads the selected failed job's own log, skips setup output,
and strips gh's job/step prefixes before choosing the first failure line. This
prevents a different failed job, or a step name containing “Failure,” from
supplying the diagnostic. A failed log download leaves no receipt and is retried.

The expanded provider regression failed on the prior implementation by retaining
an unrelated job's failure. The four CI starter/provider tests pass after the fix,
including setup before two errors and a log download failure. The shared landing
queue golden and clock tests pass, as do all 38 group/replay tests. Shell syntax,
file-size budget, swallowed-call and diff whitespace checks also pass. These are
fresh checks in this follow-up; the platform results above are from the existing
implementation commit.
