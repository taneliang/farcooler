# Engine exact-tree gate adapter (ov-1117)

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
equal checkout bytes. Publication's existing definition/configuration pins bind
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
rejection. The only capture change is passing `changes.families` to the current
lane planner, matching `lane-gate.py` (the ov-1021 API changed after ov-903).
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
