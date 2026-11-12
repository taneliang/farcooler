# Engine hooks: rules the orchestrator publishes

Design spike for ov-1109. No product code. It answers one question from the
owner (Nov 12): can a script or a prompt run at fixed points in the engine, so
that rules like "every ready ticket folds into the landing that's waiting" are
configuration the orchestrator publishes instead of code we ship?

Short answer: yes, if a hook only **decides** and the engine still **acts**.
A hook picks one move from a short, closed list for the point it's attached
to. That move is checked exactly as if the orchestrator had sent the matching
command. Its answer enters the engine's log as an input, so replay reads the
answer and never runs the hook again.

## Terms

- **Engine**: the `farcooler-workflow` crate's reducer (`step(state, input)` in
  `crates/workflow/src/reducer.rs`) and the daemon runtime that runs its effects
  (`crates/daemon/src/workflow_runtime.rs`). The reducer is pure: the same
  state and input give the same next state. The store keeps every input
  (`Exec::events`), so a workspace's state can be rebuilt from its log.
- **Point**: a moment in the engine where a choice is made today, by code or by
  the orchestrator: a ticket reaching its landing join, a group failing, and so
  on. The list below is closed; a hook can't invent a point.
- **Hook**: what decides at a point. Three kinds: a built-in rule, a script, or
  an agent.
- **Decision**: the hook's answer, a small typed JSON value from the point's
  closed list.

"Hook" already means something else here: the Claude and Codex hook files a
runner gives the agents it launches (`engine_hook_settings`,
`engine_codex_hooks`, ov-1088). To keep the two apart, the published field this
doc proposes is called `policies`, and the board and apps call them **rules**.
"Hook" is used only in this doc and in code comments.

## The rule every hook obeys

A hook adds no new power. At each point, the decisions it may give are moves
the engine or the orchestrator can already make, and each is applied through
the same checks as the command it stands for:

| Decision | Same as |
|---|---|
| join a group | `Command::Queue` (and `Leave` to move) |
| start a ticket | `Command::Start` |
| retry a failed run | `Command::Retry`, still within `MAX_RETRIES` |
| send a run back with a reason | `Command::SendBack`, within the definition's `send_backs` |
| cancel | `Command::Cancel` |
| leave it for the orchestrator | nothing: the point stays as it is today |

Two decisions are new, and each is a step the engine runs itself, never a
side effect of the hook: **rerun failed CI jobs** (a new engine verb, see
example 3) and **hold a group open** for a bounded time.

So a hook can never push, land, resolve an `Uncertain` run, approve, or review.
Those stay with their deciders (`Progression::AwaitApproval`, `Reviewer`).
A broken or hostile hook can make bad choices among legal moves; it can't make
an illegal one.

## Points and what a hook may decide at each

Each point names its **subject** (the item the decision is about), the facts
the hook is given, and the closed list of decisions. "Today" is what the engine
or the orchestrator does at that point on `main` (3f78754f9), with ov-1106 and
ov-1108 noted where their branches change it.

### 1. A ticket is ready to start

- **When**: a tracker ticket is in an eligible column and unblocked, and no
  run is going. ov-1106's scheduler checks this on each sweep.
- **Today**: the orchestrator runs `workflow start` by hand. ov-1106 adds
  `start_policy` (columns, budget, model rotation, known shared files).
- **Facts given**: the eligible tickets in board order; the runs going now and
  their models; the budget's inputs (active count, and the load sample when a
  load budget is set); declared file claims and blockers.
- **May decide**: `start` a subset of the eligible tickets, each with a model
  from its step's `models` list; `hold` (start nothing now). Never a ticket that
  isn't in the eligible list, never a model the definition doesn't list.

### 2. A slot frees

- **When**: a running effect that takes a build slot ends (a build, check,
  review), or a run leaves the budget.
- **Today**: `launch_ready` takes the outbox in order up to `effect_cap`
  (`DEFAULT_EFFECT_CAP = 4`, `workflow_runtime.rs:61`; ov-1106 removes the cap).
- **May decide**: the same as point 1. This point exists so a budget rule is
  re-asked when room appears, not only when a ticket becomes eligible. It fires
  often, so only built-in rules and scripts may attach here (see Cost).

### 3. A ticket is ready to land (joins)

- **When**: a ticket reaches its `Join { group }` step and enters
  `Phase::WaitingJoin`.
- **Today**: the orchestrator sends `workflow queue`, and `Queue` fills the
  oldest open group with room (`reducer.rs:234-268`). ov-1106's `auto_queue`
  sends it automatically; ov-1108 folds every waiting group into one.
- **Facts given**: the ticket and its candidate; the groups of that join now,
  each with phase, step, members and whether it holds the exclusive resource;
  the group `limit`.
- **May decide**: `join` the waiting group (the one at stage 0, not holding
  the resource); `join_new` (open a group behind it); `wait` (stay in
  `WaitingJoin` until the next time this point fires); `send_back` with a
  reason.

### 4. A group forms

- **When**: a `Queue` opens a new group item (`group-<revision>-<name>`).
- **May decide**: `merge_into` another waiting group of the same join, pin and
  resource; `keep`. This is how "at most one group waits" is held even when a
  second group appears some other way, such as a split.

### 5. A group fills, or would start

- **When**: a group at stage 0 is about to take its exclusive resource and
  enter its first step (`AwaitBase`).
- **Today**: it starts as soon as the resource is free.
- **May decide**: `go`; `hold_open` until a deadline the engine bounds (for
  example, wait up to 10 minutes for two more tickets that are in review). The
  deadline is kept on the group like `wait_since`, so a restart keeps it.

### 6. A group splits

- **When**: a group step's label routes to `Target::Split` (two or more
  members): today the first half keeps the group and goes back to `step`, the
  rest form a new group (`definition.rs`, `Target::Split`).
- **May decide**, for the half that does *not* keep the resource: `rejoin` the
  waiting group (ov-1108's acceptance 2), or `own_group` (today's behavior).
  For a one-member half that failed alone: `send_back` with the failure as the
  reason, `retry` alone, or leave it for the orchestrator.

### 7. A ticket leaves a group

- **When**: `Leave`, or a cancel that detaches a joined ticket.
- **May decide**: `rejoin_later` (back to `WaitingJoin`, so point 3 fires
  again); `send_back` with a reason; nothing.

### 8. A CI result arrives

- **When**: an `AwaitCi` step's effect answers: `passed`, `rejected`, `timeout`
  or `cancelled` (`emits_unrouted`, `definition.rs:449-459`).
- **Facts given**: the CI receipt the step already writes (run id, conclusion,
  and, new here, the failed job names and attempt number from the same
  `gh api` call).
- **May decide**: `accept` (the label routes as published); `rerun_failed`
  (the engine reruns the failed jobs of that run, and the step waits again);
  `relabel` to another label the step's routes declare. A hook can never turn a
  `rejected` into `passed`: the only way from red to green is a rerun that
  comes back green.

### 9. A run fails

- **When**: an item enters `Phase::Failed`: a step spent its attempts, or a
  route ended at `Fail`.
- **Today**: it waits on the needs-you list for the orchestrator to `retry`,
  `send-back` or cancel.
- **Facts given**: the failed step, its `failure` sentence (ov-1058), attempts
  and retries spent, the last evidence, and the log tail path.
- **May decide**: `retry` (within `MAX_RETRIES`); `send_back` with a reason;
  `cancel` with a reason; `leave` for the orchestrator.

`Uncertain` is never a hook point. An uncertain run may have done something
outside (a push whose result wasn't seen); only a person or the controller's
`Resolve`, with the evidence, may settle it.

## Hook kinds compared

| | Built-in rule | Script hook | Agent hook |
|---|---|---|---|
| What it is | A named rule with typed settings, in the published definition: `{"fold": "all_waiting"}` | A pinned command. Typed JSON in on stdin, one typed decision written to `$FARCOOLER_RECEIPT` | A prompt to an agent the engine starts, with a model from a `models` list. Its decision is the receipt `farcooler workflow decide` writes, never its prose |
| Where it runs | Inside the reducer, during `step` | A runtime effect, like a `Run` gate or the CI look | A runtime effect, like `Reviewer::Agent` (ov-969) |
| Determinism | Total: a pure function of state and the published definition | The engine can't see inside it. It's deterministic *to the engine* because its answer is recorded once and read back | Not deterministic at all; same as a script once recorded |
| Replay | Free: replay runs the same rule on the same state | Replay reads the recorded `Decide` input; the script never runs again | Same as a script |
| Cost per firing | None measurable | One process: milliseconds to seconds of CPU | An agent turn: tokens and 1-10 minutes |
| Latency | None: decided in the same step | The point waits until the receipt or the timeout | Minutes; the point waits |
| Failure | Can't fail at run time: settings are checked at publish | Exit without a receipt, bad JSON, an illegal decision or a timeout: the fallback applies | Same as a script, plus a turn that ends with no decision |
| Who can change it | The orchestrator drafts, the owner publishes (today's draft flow) | Same, and the script file is pinned by digest like `LaunchFile` | Same, and the prompt is pinned |
| Good for | Rules we can name: fold, budgets, rerun named jobs | Rules that need facts the engine lacks: memory pressure, a flaky-jobs list kept in the repo | Judgment: read a failure log, decide retry or send back with a reason |

The order matters: a rule should be a built-in when it can be named and its
settings typed. Scripts are the escape hatch for facts outside the engine.
Agents are for judgment, at points that fire rarely and where a person or the
orchestrator decides today (points 8 and 9).

## Keeping a decision deterministic and replayable

The reducer stays pure. A script or agent hook never runs inside `step`; it
runs as an effect, and its answer comes back as an input. This is the same
shape every effect has today (an `Observe` with `Evidence`), with one more
command.

### Firing

At the end of a `step`, the reducer checks each point that the step reached
(the same place ov-1108's `consolidate_groups` and the ready-items pass run).
For a point with a built-in rule, it applies the rule right there. For a
script or agent hook, it:

1. Sets `deciding` on the subject: `{ call, policy, point, fence,
   since_ms, deadline_ms }`. `call` is `hook-<revision>-<policy>-<subject>`.
   While `deciding` is set, the engine doesn't take the point's default
   action. `deadline_ms` is `now_ms + timeout_ms` from the input, so a restart
   keeps the same deadline (as `wait_since` does for waits).
2. Emits one `EffectKind::Decide { policy, point }` effect, carrying the hook's
   **input document** (below), fenced like every effect.

At most one call is open per subject and point. A second firing while one is
open is dropped. `max_calls` (per subject, per run; default 3, published) stops
a hook from being asked forever at a point that keeps firing.

### Input document

Built by the reducer from state at the firing revision, so it's the same on
every replay:

```json
{
  "schema": "hook-input/v1",
  "point": "ticket.ready_to_land",
  "call": "hook-00000000000000004821-landing-ov-1040",
  "revision": 4821,
  "now_ms": 1794470400000,
  "subject": { "item": "ov-1040", "candidate": "<sha>", "join": "landing" },
  "facts": { "groups": [ { "id": "group-...-landing", "phase": "Blocked", "stage": 0,
    "holds_resource": false, "members": ["ov-1048"] } ], "limit": 0 },
  "allowed": ["join", "join_new", "wait", "send_back"]
}
```

Only the facts listed for the point are given: no tokens, paths outside the
workspace, or other workspaces' items. `allowed` lists the decisions open right
now, so a script doesn't have to work out legality.

### Output and receipt

The runtime runs the script with the input on stdin, in the workspace's
checkout at the base commit, read-only by contract, with
`FARCOOLER_RECEIPT` set. The script writes one decision there:

```json
{ "schema": "hook-decision/v1", "call": "hook-00000000000000004821-landing-ov-1040",
  "decision": "join", "group": "group-00000000000000004790-landing" }
```

Stdout and stderr go to `receipts/<effect>.log`, as for checks today. The
runtime then submits:

```text
Command::Decide { item, call, fence, decision, evidence }
```

`evidence.manifest` is the digest of (binding, input document, raw receipt
bytes); `evidence.receipt` is `digest("hook-receipt/v1", manifest)`;
`evidence.policy_pin` is the adapter configuration pin, as every effect
carries today. So the log holds the input, the exact decision and a digest
that ties them to the receipt file on disk.

### Validation

The reducer accepts a `Decide` only if all of these hold, in this order:

1. **Fresh**: `call` and `fence` match the subject's `deciding`. If not, the
   subject moved on (the orchestrator queued it by hand, a cancel came first),
   and the decision is refused as `Stale` and dropped. Nothing else changes.
2. **Pinned**: the policy pin on the call is the one published when it fired.
   A republish while a call is open doesn't change what that call may decide.
3. **Well-formed**: the decision parses as the point's closed type, with
   unknown fields refused (`deny_unknown_fields`) and sizes bounded (a reason
   is checked with `valid_rework_reason`, 4096 bytes).
4. **Legal now**: the decision is in `allowed`, and the matching command's own
   checks pass (a `retry` within `MAX_RETRIES`, a `join` into a group that is
   still waiting, a model the step lists).

A decision that is fresh and pinned but fails 3 or 4 is not dropped: the
fallback applies, and the subject records why (`hook_refused`, a sentence the
card shows, like `failure` today). The point never stays stuck.

### Timeout, crash and fallback

Every policy names a `fallback`, which must be a built-in decision legal at
that point. Left out, it is `leave`: the engine does what it does today without
the hook.

- **Timeout**: the runtime stops the script at `deadline_ms` and submits
  `Decide { decision: timeout }`. The reducer applies the fallback. The timeout
  is itself an input, so replay applies the same fallback at the same revision.
  Bounds: script `timeout_ms` at most 10 minutes (default 30 s); agent at most
  30 minutes (default 10 min).
- **Crash or no receipt**: a script exit with no receipt is no answer. Because
  a hook only reads, the runtime runs it once more, as the `Route` verb does
  today ("it only reads, so a crash before its fact is written reads again").
  A second failure applies the fallback.
- **Daemon restart**: an open call whose receipt exists is settled from it; one
  without a receipt is run again until the deadline. A hook has no outside
  effects, so a call can't become `Uncertain`.
- **Bad output**: the fallback, with `hook_refused` saying what was wrong.

### Facts from outside the engine

A built-in rule that reads something outside the state (the load average for
ov-1106's load budget, CI for point 8) must get it on the input, the way the
store stamps `busy_resources` on every input today. Otherwise the log says
*that* a ticket started but not *why*. ov-1106's branch reads `getloadavg`
in the runtime's scheduler and sends a plain `Start`; the sample isn't kept.
Follow-up below: stamp the sample on the `Start` input, so the board can say
"started at load 4.1, under 6".

### Cost and where each kind may attach

| Point | Fires | Built-in | Script | Agent |
|---|---|---|---|---|
| 1 ready to start | per eligible ticket per change | yes | yes | no |
| 2 slot frees | per effect end | yes | yes | no |
| 3 ready to land | per ticket | yes | yes | no |
| 4 group forms | per group | yes | yes | no |
| 5 group would start | per group | yes | yes | no |
| 6 group splits | per failed group | yes | yes | yes |
| 7 leaves a group | rare | yes | yes | no |
| 8 CI result | per CI run | yes | yes | yes |
| 9 run fails | rare | yes | yes | yes |

Agent hooks are refused at publish on points that fire per ticket or per slot:
each ticket passes points 1 and 3 at least once and ends four or more slot
effects (build, check, review, and each send-back's rebuild), so on Nov 12's
11 landing tickets that's at least 50 agent turns spent on choices a rule can
make. Hook effects don't take build
slots, like `AwaitBase` and `AwaitCi` today (the `wait` exemption in
`launch_ready`), so a slow hook can't starve builds and a full budget can't
stall a hook. An agent hook counts against ov-1106's agent budget like any
other agent.

## Worked examples

These are fragments for the published definition. `policies` is a map from a
policy name to its point, its hook and its fallback.

### 1. Fold every ready ticket into the waiting group (ov-1108)

```json
"policies": {
  "fold-ready": {
    "point": "ticket.ready_to_land",
    "join": "landing",
    "rule": { "fold": "all_waiting" }
  },
  "one-waiting": {
    "point": "group.forms",
    "join": "landing",
    "rule": { "merge": "into_waiting" }
  },
  "split-rejoins": {
    "point": "group.splits",
    "join": "landing",
    "rule": { "rest": "rejoin_waiting", "alone": "send_back" }
  }
}
```

With `groups.landing.limit: 0` (unlimited, ov-1108). Group A gates; tickets
reach their join at 10:02, 10:09 and 10:20. Each time point 3 fires, `fold:
all_waiting` decides `join` into the one group at stage 0 that doesn't hold the
resource, or `join_new` if there is none. When A ends, B takes the resource
with all three: one fold, one gate, one CI watch. If B fails and splits, the
half that doesn't keep the resource rejoins whatever group is waiting by then,
and a single ticket that failed alone is sent back with the failure as its
reason.

This is ov-1108's `consolidate_groups`, named and published. ov-1108 is
built as a fixed rule now (the manager's ruling); slice 1 below moves it onto
this shape with no change in behavior.

### 2. A load-based start budget (ov-1106)

The built-in form is ov-1106's `start_policy`, moved under `policies`:

```json
"policies": {
  "start": {
    "point": "ticket.ready_to_start",
    "also": ["slot.frees"],
    "rule": { "budget": { "load_below_milli": 6000 }, "columns": ["todo"], "rotate_models": true }
  }
}
```

A script variant, for a rule the engine has no facts for (say, also hold
starts while free disk is under 20 GB, because lane `target/` folders take
5-20 GB each):

```json
"policies": {
  "start": {
    "point": "ticket.ready_to_start",
    "also": ["slot.frees"],
    "script": "start-budget",
    "timeout_ms": 5000,
    "fallback": { "decision": "hold" }
  }
}
```

`start-budget` is a pinned command in the adapter configuration, like a check
(`CodingCommand { argv, environment, timeout_ms }`). It's given the eligible
tickets, the runs going, and the load sample, reads free disk itself, and
writes `{"decision": "start", "tickets": [{"item": "ov-1201", "model":
"codex:gpt-5.5"}]}` or `{"decision": "hold"}`. If it hangs, the fallback holds
starts: the safe side of a budget. Its disk reading is recorded in the
receipt log, so the board can say why it held.

### 3. Retry a known-flaky CI job once

Built-in, on the landing group's CI step:

```json
"policies": {
  "flaky-ci": {
    "point": "ci.result",
    "step": "ci",
    "rule": { "rerun_failed": { "jobs": ["iOS UI tests"], "times": 1 } }
  }
}
```

When the CI step answers `rejected`, the rule reads the failed job names from
the CI receipt. If every failed job is in `jobs`, and this run hasn't been
rerun `times` already, it decides `rerun_failed`. The engine then runs a new
`RerunCi` verb (`gh run rerun <id> --failed`, recorded like the CI look) and
the `AwaitCi` step waits again on the same commit. Any other failure, or a
second red, routes `rejected` as published. The rerun count is kept on the
item (like `back_edges`), so a restart can't rerun twice.

The rerun is an engine verb, not something the hook does, for two reasons: a
rerun is an outside side effect and needs the same receipt-and-reconcile care
as `Push`; and a hook that only decides can be dropped or replayed safely.

An agent variant, for when the flaky list isn't known ahead: `"agent":
"ci-triage"` with a prompt that reads the failed jobs' logs and decides
`rerun_failed` or `accept`, `max_calls: 1`, `fallback: accept`. It costs an
agent turn per red CI run, which tonight's numbers don't justify yet.

## Recommendation

Build hooks in three slices, smallest first, and stop after slice 1 if the
built-in rules cover what the orchestrator does by hand.

1. **Rules are published policies** (ov-1112, built-ins only). Add `policies` to the
   definition with the nine points as a closed enum and built-in rules for
   points 3, 4 and 6 (ov-1108's fold, merge and split), 1 and 2 (ov-1106's
   start policy), and 9 (`retry` up to N, then `leave`). Each built-in decision
   is recorded on its subject as a `Decision` entry (point, rule, revision,
   input digest, decision), so the board says "joined group 463 by fold-ready".
   This moves ov-1106 and ov-1108 onto one shape and adds no new kind of
   effect. Most of the value is here: it replaces every hand-run `workflow
   queue`, `leave` and `retry` the manager did on Nov 12.
2. **Rerun a known-flaky CI job once** (ov-1113): the `RerunCi` verb and the
   `rerun_failed` built-in at point 8. Separate because it's a new outside
   side effect with its own reconcile story.
3. **Script hooks at the rare points** (ov-1114): `EffectKind::Decide`,
   `Command::Decide`, the input and decision schemas, receipts, timeouts and
   fallbacks, first at points 6, 8 and 9. Then points 1 to 5 once the
   firing rate is measured.

Agent hooks wait until a script hook has run for a while: they reuse the
`Reviewer::Agent` machinery (`farcooler workflow verdict` becomes `farcooler
workflow decide`), and nothing tonight needs judgment a rule can't give.

Separately, small: stamp the load sample on a scheduled `Start` input (ov-1115).

All of it lives in the Rust engine: the decision types and their validation in
`crates/workflow`, firing and fallback in the reducer, script and agent
execution in the daemon runtime. Apps only show a subject's decisions and
`hook_refused`, as they show `failure` today.
