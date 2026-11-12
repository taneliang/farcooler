# Rerunning a flaky CI job

A published schema 2 definition can give an `AwaitCi` step a single retry for
named jobs (ov-1113). The rule belongs to the definition's `policies` map:

```json
"policies": {
  "flaky-ci": {
    "point": "ci.result",
    "step": "ci",
    "rule": { "rerun_failed": { "jobs": ["iOS UI tests"], "times": 1 } }
  }
}
```

`step` names an `AwaitCi` step, including a group's step. The group inherits
its matching rules when formed. Job names match exactly. `times` must be 1;
empty lists, duplicate names, other step kinds and unknown rule fields are
refused. The optional `fallback: "accept"` keeps the result's published
route, as does leaving it out. It never marks red CI as passed.

The CI adapter's pinned environment must name its absolute `CI_GH` executable.
The engine adds `run rerun <id> --failed --repo <repository>` itself, using that
environment. The CI look command only reads. The runner's suggested adapter
reads the run's `run_attempt` and all failed job names from that exact attempt,
including every page of jobs. A custom adapter writes the same receipt:

```json
{
  "binding": "<FARCOOLER_OPERATION_BINDING>",
  "repository": "owner/repository",
  "run": {
    "id": 73,
    "run_attempt": 1,
    "name": "CI",
    "head_sha": "<FARCOOLER_CANDIDATE>",
    "event": "push",
    "status": "completed",
    "conclusion": "failure",
    "html_url": "https://github.com/owner/repository/actions/runs/73",
    "created_at": "2026-11-13T00:00:00Z"
  },
  "failed_jobs": ["iOS UI tests"]
}
```

Old receipts without attempt or failed-job facts still route their CI result;
they cannot authorize a rerun. A receipt's binding, repository, workflow, event
and candidate are checked before its facts reach the rule.

When every failed job is listed, the reducer spends the count on the item and
emits a separate `RerunCi` effect. That effect records its send intent before
calling GitHub, then writes its own receipt. Success resumes `AwaitCi` on the
same commit. The adapter receives `FARCOOLER_CI_RUN` and must look at that run;
only a strictly newer attempt can finish the wait. A second red or an unlisted
failure follows the step's original rejection route. Its wait deadline starts
again after the rerun is accepted; restarting a wait retains that deadline.

Recovery never sends the rerun again. An existing receipt settles it; without
one, recovery reads the run and accepts only a newer attempt of the same run,
commit, workflow and event. If GitHub cannot confirm the send, the effect stays
`Uncertain`, with its count spent. Replaying the event log reads the recorded
CI facts and reproduces the same decisions without contacting GitHub.

A group's CI rejection still fails the group. As with other CI failures after
a push, tickets keep the commit that already landed; rerunning CI cannot undo
a push or send those commits back through landing.
