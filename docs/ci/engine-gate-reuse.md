# Engine exact-tree gate adapter (ov-1117)

The completed isolated canonical wire/doc validation and independent
adapter/prerequisite review are recorded in
[engine-gate-operational.md](engine-gate-operational.md). Historical outstanding
items and failed attempts below describe their status at that time. Reuse on
existing production workflows remains disabled.

The configured engine already executes Check and Run commands with a cleared,
pinned environment, retains their output and numeric exit in a binding-specific
ProcessFact, and checks the candidate after execution. `scripts/engine-gate.py`
is an opt-in command for those stages. It runs the canonical lane plan through
ov-903's prospective capture and checker. It does not cache arbitrary configured
commands, reviews, builds, pushes or remote CI verdicts.

## Publication

Publish a configured check whose argv is an absolute Python executable followed
by the absolute checkout path to `scripts/engine-gate.py`, then:

```
--inputs /private/path/reviewed-public-inputs.json
--evidence-root /private/path/engine-gate-evidence
--trust-scope runner-name:public-configuration-epoch
```

Set PATH and HOME explicitly in the configured command environment, together
with the public SDK/toolchain paths the reviewed declaration needs. The engine
supplies candidate, resolved base, definition pin, policy pin, workspace and
invocation binding. An invocation outside that engine contract is refused.
Use the script from the candidate checkout; ov-903 checks that runner bytes
equal checkout bytes, and the adapter refuses execution from another path.
Publication's existing definition/configuration pins bind
the command argv, timeout and configured environment.

The default executes every required job. Add `--enable-reuse` to the published
command only after independent review of this adapter **and the concrete public
prerequisite inventory**, followed by a clean operational application gate and
repeat measurement. This change does not enable reuse on any existing workflow.
The trust scope names the runner and external configuration epoch. Change it
when global configuration, service assumptions or undeclared external inputs
change. Do not capture secrets or global configuration to manufacture this ID.

## Provenance and recovery

The adapter leaves ov-903's input schema/checker intact. It appends a private
public context file and its own executable source to each job's native input
inventory. The context binds the workflow definition, policy, workspace, base,
reviewed inventory bytes/path, adapter source and explicit trust scope. The
definition pins the canonical configured command. Source HEAD/tree, tracked
content and modes, clean index/status, nonignored untracked files, commands,
target/features in the plan, effective restricted public environment, resolved
toolchains, SDK/cache/native inputs and host are captured by ov-903.

Different engine invocation bindings may reuse results within that exact scope;
the new engine ProcessFact still names the new invocation. A different base,
workflow, policy, board, trust scope, source, plan or environment cannot hit.
Unknown/incomplete/failed jobs rerun as whole jobs. Valid successful jobs can
survive a retry of another job. Full hits revalidate the entire evidence chain,
then resample current scope while holding the capture lock. Full/partial misses
use the original prospective writer under that same lock. Neither a v1 summary
nor an engine process exit can supply reusable job coverage.
The locked scope must still name the engine candidate. The writer also compares
its run-start scope with that lookup scope before creating an attempt or running
any job; acquiring the evidence lock does not prevent another process moving HEAD.

All attempts, failures and raw logs remain immutable. When evidence cannot supply
coverage, the exact previous pointer is quarantined privately with its digest;
the new capture records that archive reference and executes every required job.
No corrupt chain is imported or repaired. Earlier raw files remain in place for
investigation, including known failures. Nothing deletes build output or another
task's work. Heavy jobs retain the existing shared queue and wait at
one-minute load >=20.

Each invocation prints and privately saves observational JSON with required,
reused and selected job counts, elapsed wall time, scope digest, binding and
numeric exit. Metrics never authorize a pass. Report aggregate job hit rate as
sum(reused)/sum(required); compare cold and repeat wall times on the same clean
scope. Wall-time differences include adapter/checker overhead and are not an
estimate of application build savings or queue time avoided.

## Audit and validation limits

The five ov-903 source/doc files were restored from reviewed commit
`3334a6c6578411ccd38d5687e92c2ca89ef72221`, preserving its v2 protocol and v1
rejection. The capture changes pass `changes.families` to the current lane planner,
matching `lane-gate.py` (the ov-1021 API changed after ov-903), and add an
optional expected-scope guard for the engine adapter. Standalone capture keeps
its existing behavior; the v2 schema and checker are unchanged.
The current CI workflow is preserved, with the original and adapter fixtures
added to its wire job. Historical Gate4/5 receipts remain ineligible.

Adapter fixtures launch real subprocesses in clean committed disposable
repositories with tiny canonical shell jobs. They cover failed-job retry, full
hits, default-disabled execution, immutable historical bytes, workflow/policy/
workspace/trust/environment misses, dirty/untracked/candidate/context refusals,
v1 rejection and corrupt logs. The original 16 fixtures and four guard mutations
exercise partial coverage, exit values, prerequisites, hidden source changes,
interruption and capture/checker concurrency.

The clean operational fixture measures two real shell jobs and their repeat;
it is not an application gate or evidence that a production prerequisite
declaration is complete. Independent adapter/declaration review and a real clean
application run remain required before production enablement. ov-903's historical
source review does not approve this adapter or any new inventory.

## Working-tree validation (2026-11-13)

- Original real-process suite: 16 passed. Adapter suite: 5 passed, including the
  independent summary checker after the clean repeat.
- Four original guard mutations produced expected assertion failures. Removing
  the adapter's policy binding independently produced an invalid two-job hit and
  the new mismatch test failed as intended.
- Lane planner and summary checker self-tests passed. Their disposable commit
  fixtures used invocation-local `commit.gpgsign=false`; no global settings
  changed. File-size budget, protocol lint and diff whitespace checks passed.
- Final measured clean shell pair: cold 0.980 s, repeat 0.730 s, observed net
  wall saving 0.251 s. Repeat job hit
  rate 2/2 (100%); aggregate rate across the pair 2/4 (50%). These tiny jobs do
  not justify projecting the historical 2,806 s application-gate opportunity.
- One-minute host load samples stayed below 20 (5.15–13.85). No application
  builds, target cleanup, real prerequisite declarations or existing operational
  gate reuse occurred. Only task-owned fixture subprocesses were started.

Acceptance 3's real application operational run and independent adapter/input
review are outstanding; production reuse remains disabled. The fixture result
is deliberately not represented as that operational acceptance evidence.

## Follow-up audit (2026-11-13)

Three additional real-process regressions exposed provenance gaps: an adapter
outside the candidate checkout could execute, HEAD could move between the early
snapshot and locked lookup, and the writer could resample a different scope
between lookup and execution. Each test failed by assertion on the previous
implementation. The adapter now enforces its checkout path and locked candidate;
the prospective writer refuses a different expected run-start scope before
writing a pointer or executing a job. The original ov-903 checker remains the
coverage authority.

Validation: eight adapter tests and sixteen original capture/checker tests pass;
all four original guard mutations still produce assertion failures. The planner
and summary checker self-tests use invocation-local signing overrides for their
disposable commits. No global Git configuration is changed.

A real canonical wire/doc gate was attempted with reuse disabled in the clean
isolated checkout `/private/tmp/ov1117-operational-_skxgmbc/repo` (base
`bb38bf01b8f1583db1e182d177111dc5febce35d`). It failed: the declared PATH selected
an x86-only `/usr/local/bin/python3` that child processes could not execute
(exit 126, “Bad CPU type in executable”). The v2 capture correctly stayed red;
its private manifests and raw logs remain in the sibling `evidence/` directory.
This attempt preceded the final writer-scope guard and is diagnostic evidence,
not validation of the final candidate. Host load rose above 20 during the
attempt, so its elapsed time and the concurrent fixture timings are excluded
from the ticket's performance measurements. No repeat or production reuse was
enabled, and no other task's work was stopped.

Acceptance 3 remains open: no independently reviewed concrete application input
inventory was supplied, no successful clean application gate has been captured,
and there is no new valid operational hit-rate/wall-saving measurement under
load 20. The prior shell-fixture measurements above retain their stated scope.

## Malformed receipt recovery (2026-11-13)

A new subprocess counterexample replaces a receipt with JSON `null` and updates
the pointer's manifest digest. Previously ov-903's validator raised
`AttributeError` while accessing that receipt, bypassing the adapter's miss
handler. The adapter now treats this malformed shape as unusable evidence,
archives the exact pointer, and executes every required job. The test verifies
zero reuse, successful independent summary checking after the fresh capture,
and byte-for-byte preservation of the corrupt manifest and all prior raw files.
It failed with the original uncaught exception before the fix. The prospective
writer, checker, schema, and production opt-in are unchanged.

The adapter suite now has nine passing tests; all sixteen original ov-903
tests also pass. Fixture timing is excluded from operational measurements.

A fresh canonical wire/doc preflight with the ARM Python executable was attempted
in `/private/tmp/ov1117-canonical-asljch2j`, with reuse disabled and a private
empty HOME. It refused execution because the restricted PATH lacked Cargo,
which the wire job needs for its conditional typeshare installation. No job
receipt or success was produced. The next isolated candidate, in
`/private/tmp/ov1117-canonical-2ka9te8p`, includes the installed Cargo and
typeshare paths, but its launcher refused to start at one-minute load 24.00.
Both private directories retain their invocation/environment/input declarations;
these declarations have not received independent prerequisite review.

For the next operational attempt, select `/opt/homebrew/bin/python3.14` explicitly
and put `/Users/e-liang/.local/bin` before `/usr/local/bin` so child `python3`
commands use the ARM interpreter too. Include the installed Cargo and typeshare
paths even though the selected jobs are light. Independently review the concrete
tool/dependency inventory before allowing a repeat to reuse results. A clean
canonical success and repeat measurement under load 20 remain outstanding;
there is no new operational hit rate or saved-wall-time claim.

The operational follow-up also found that relocating Xcode's Git through a
symlink changes its default helper/template lookup. Put its actual installation
directory on PATH after the curated Python/tool bin, rather than symlinking Git
into that bin. Preserve the failed attempt and start a new default-disabled cold
capture after correcting the environment; its earlier red receipts cannot
authorize a repeat.
