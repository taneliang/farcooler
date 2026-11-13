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

## Current Plan screens

The Mac and iPhone now render the groups in their current Plan columns, which
previously consumed only `view.column`; the older sections views were insufficient
to make the queue reachable from today's home screen. Both columns keep the full
Rust answer and lay out the same shared group row. Queue-only plans also bypass
the empty notice in the iPad sections view and Android. No group ordering, words,
member selection, timing, or failure interpretation moved into a renderer.

The native regressions adapt the shared queue golden to the runner's required
Apple card fields, with no themes or line tickets. The Mac home is checked at
320 pt; the iPhone test checks every supplied line and its horizontal bounds.
The existing shared SwiftUI wrapping check now compares long and short titles
instead of assuming a fixed height across system font metrics. Android's 320 dp
capture likewise exercises the queue without themes or line tickets.

Fresh verification of this follow-up:

- Rust queue golden and clock tests: 2 passed.
- Mac Plan column and sections suites, linked against freshly built Rust cores:
  21 passed. The valid new home regression fails with the original column restored.
- Android unit suite passed; all 17 Compose Plan section tests passed, including
  the 320 dp queue with long titles, split details, and gate failure lines. The
  queue test fails with the old empty-state guard restored. Light and dark
  captures were generated and the light capture was checked; the Rust JNI library
  was loaded by absolute path.
- File-size, visual-token, copy, swallowed-call, doc-comment and diff whitespace
  checks passed.
- iPhone queue regression passed on the actual current Plan screen: all four
  groups, each complete supplied line, and horizontal bounds on a phone. The
  corrected focused test ran in 18 seconds with no skips.
- The iPhone Plan class and per-push smoke set were also run. All selected
  smoke tests passed, as did the other Plan navigation, ordering, ticket/key,
  and largest-accessibility-type checks. `testEachFooterCountOpensItsTickets`
  exceeded Xcode's six-minute timeout while searching absent footer counts.
  The broader run also contained the earlier invalid queue fixture (missing
  the Apple card's required status); its final corrected regression passed
  separately as recorded above. The broader run is therefore not reported
  as green. No footer behavior was changed in this ticket.

## Automated web narrow-width regression

The web Plan browser check now also renders the shared landing queue golden
without themes, line tickets, or pipeline agents. It checks that the workflow
reaches the presenter and that the current Plan screen draws every group and
complete line in the supplied order. At 320 px it checks actual text bounds,
clipping, and nonzero group heights, with the work screen visible and the
production Plan CSS applied.

Fresh verification: the browser check passed, all 287 web unit tests passed,
and both Rust queue golden/clock tests passed. Disabling the production queue
render block made the browser regression fail for the missing Landings section
and four missing groups; the source was restored afterward. File-size and diff
whitespace checks passed. The initial sandbox-only Chrome launch and Rust
dependency build failed; both passed with the local process/compiler access
their setup requires. Native UI code was unchanged in this follow-up; its
previous platform verification is recorded above.

## Real landing-group validation

The authenticated Plan read now validates engine groups with `validate_group()`,
matching the reducer. Ordinary `validate()` rejects their AwaitBase, Fold and
Push stages, so the optional workflow enrichment previously failed the whole
Plan request as `workflow Invalid` when an actual landing group existed.

The new socket regression builds a valid landing definition with all five group
verbs, seeds its member and queue through the durable reducer and effect admission,
and reads the group through the same authenticated `plan.get` dispatch used by
the apps. It then feeds that answer into the shared Plan presenter and checks the
supplied step and member. The test explicitly confirms that the group definition
requires group validation. It failed with `Protocol("workflow Invalid")` before
the fix and passed afterward. All application behavior remains in Rust.

Fresh verification: all 351 client unit tests passed (one existing ignored test),
including the queue golden, real group socket read and workflow session tests;
all 144 workflow tests passed. The complete client suite totals 435 passing tests
and one existing ignored test. Its daemon integration target passed all 31 tests
after building its required CLI binary and removing the parent agent session’s
`FARCOOLER_NATIVE_HOOK_SOCKET` from the test environment; that variable overrides
the tests’ private hook sockets. File-size, swallowed-call, copy, staged doc-comment
and diff whitespace checks passed. No platform layout code changed in this round.

## CLI enrichment and final acceptance audit

The Mac's `plan --json` read now uses the same authenticated workflow request,
decoding, group validation, and compatibility policy as the Session-based apps.
The socket integration regression invokes the actual CLI, compares its output
with `Session::plan_with_workflow`, and holds both to the fixture consumed by
the Mac's 320 pt Plan home test. The Mac test no longer injects workflow data,
so it cannot conceal a missing CLI read.

The final audit found the shared queue reachable in the current Mac and iPhone
Plan columns, the iPad and Android Landings sections, and the web Plan screen.
Each renderer lays out the Rust-supplied group title and complete lines; member
selection, failure interpretation, elapsed time, and ordering remain in Rust.

Fresh verification on November 13: all 287 web tests, the client TypeScript
check, and the production Plan browser regression passed. The browser check
verifies all four queue-only groups and their complete lines at 320 px,
including horizontal bounds and clipping. File-size budget, copy,
swallowed-call, visual-token, and diff-whitespace checks also passed.

The actual CLI socket regression passed, including equality with the shared
Session read and the Mac fixture. All three AgentKit PlanView tests passed
against a freshly built Rust client library. Platform UI code was unchanged in
this audit; the Mac, iPhone, and Android layout results above remain the prior
verification, rather than fresh runs in this pass.

The client unit suite passed 350 tests, with one existing ignored test; its
remaining two tests initially lacked their required `target/debug/farcoolerd`.
After building that binary, both passed on a focused rerun, for 352 passing
client tests across the runs. Initial sandbox-only socket, browser-process,
and compiler-cache setup failures were resolved with the required local access.
This pass changes only this verification report; the implementation and its
CLI regression were already committed on the ticket branch.
