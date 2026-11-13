# Script rules at engine decision points

A published workflow definition can include `script_policies`. Each rule names one of
`group.splits`, `ci.result`, or `run.fails`, and pins a script's absolute path
and bytes. Only one rule may name each point. Existing definitions omit this
field and retain their pins and behavior.

```json
"script_policies": {
  "failure-triage": {
    "point": "run.fails",
    "script": "/home/runner/rules/triage.sh",
    "digest": "wf1:<64 hex digits>",
    "timeout_ms": 30000,
    "max_calls": 3,
    "fallback": { "decision": "leave" }
  }
}
```

The digest is `farcooler_workflow::digest(&script_bytes)`, the SHA-256 of the
canonical JSON byte array, prefixed with `wf1:`. The definition pins the entire
policy, including the timeout and fallback. Scripts run directly (include a
shebang), from a private copy of those verified bytes, with an empty environment
apart from `FARCOOLER_RECEIPT`. Their working directory is a detached checkout at
the run's pinned base. Scripts have a **read-only contract**: they inspect facts
and write their receipt; the engine performs the chosen move. This contract is
not an OS sandbox. Stdout and stderr are retained beside the receipt.

The reducer writes `hook-input/v1` to stdin. It includes the point, call ID,
revision, firing time, subject, fence, policy pin, candidate, failure sentence,
last evidence, CI result label, members, retries, related groups and decision
names. No other workspace's items or credentials are included. The call and its
input are retained on the subject and the effect. Group definitions inherit the
published rules, so splits and CI use the same pins as their tickets.

The script writes one JSON object to `$FARCOOLER_RECEIPT`, at most 16 KiB:

```json
{
  "schema": "hook-decision/v1",
  "call": "<call from stdin>",
  "decision": "retry"
}
```

Decisions at each point:

| Point | Decisions | Fallback |
| --- | --- | --- |
| `group.splits` | `own_group`; `rejoin` with `group` | `own_group` |
| `ci.result` | `accept`; `relabel` with `label` | `accept` |
| `run.fails` | `leave`; `retry`; `send_back` or `cancel` with `reason` | `leave` |

Rejoin requires a waiting group of the same definition, adapter pin and
resource, with room for the members. Retry, send-back and cancel pass the
ordinary orchestrator command checks, including their budgets and landing
restrictions. Relabel requires a declared route and can never turn red into
`passed`. A CI observation waits for its rule and then routes its original
receipt (or the legal replacement label). `Uncertain` is never a rule point.
CI observations must pass the ordinary phase, authorization, evidence and result
label checks before a rule can suspend them or spend its call budget.
There are no agent rules or CI rerun verbs in this slice.

Call and fence freshness are checked before parsing; a superseded call is
stale. A fresh, authenticated answer with a mismatched policy pin applies the
fallback without retrying. Duplicate fields, unknown schemas,
decisions or fields, malformed JSON, and moves that are illegal now apply the fallback and leave a
sentence in the run's failure context. The fallback is deliberately limited to
the point's default action, so it always works when another actor changes the
state while a script runs.

Each attempt is admitted durably before spawning. Timeouts and exits without a
receipt become explicit `Decide` inputs. The first emits a second attempt; the
second applies the fallback. Each attempt keeps its deadline across restarts,
and a retry gets one new timeout allowance. Timeouts are bounded to ten minutes,
and a policy may fire at most three times per subject's run. An attempt's receipt
is bound to its effect, input and raw bytes by its manifest digest. A restart
reads the journal or receipt before spending a missing attempt; it never turns a
read-only call into an uncertain operation. Replay reads only the logged inputs.

Rule effects hold no build slot and may run even when the build cap is zero.
A split half waits for its decision before taking the group's resource. A
controller's legal manual move supersedes an open call, fencing its late answer.
