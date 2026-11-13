# Automatic workflow scheduling

The owner can add `start_policy` to the definition in an existing publication
bundle, then publish it with `farcooler workflow publish <workspace> --file
<bundle.json>` and the usual revision, epoch, sequence, command ID and expected
pin. Publishing remains an owner action. A proposal does not enable scheduling.
No app update or daemon restart is needed to change the policy.

For example, these fields on a coding definition start up to three unblocked
`todo` tickets, rotate the models listed for each agent step, and queue tickets
that reach their landing join:

```json
{
  "start_policy": {
    "columns": ["todo"],
    "budget": {"Fixed": 3},
    "rotate_models": true,
    "auto_queue": true,
    "files": {
      "ov-1": ["crates/core/src/title.rs"],
      "ov-2": ["crates/core/src/title.rs"]
    },
    "dependencies": {"ov-3": ["ov-2"]}
  }
}
```

This is a fragment to add to the definition, not a complete publication.
`models` continues to map step IDs to allowed `harness:model` lists. Automatic
starts pick successive entries from each list when `rotate_models` is true;
manual starts can still select a listed model explicitly.

`budget` also accepts `"Unlimited"`, or `{"Load":{"below_milli":6000}}` to
start only while the machine's one-minute load average is below 6. Missing load
samples hold starts. Load-based admission leaves at least 60 seconds between
successful starts across all boards, including immediate pump re-entry and
one-second service scans. Set a different positive gap in the published budget:
`{"Load":{"below_milli":6000,"min_start_gap_ms":120000}}`. The longest
published gap applies to the whole runner, alongside the strictest load and
fixed limits. Failed admissions and queues do not consume the gap. The timer
is recovered once from accepted automatic start events on the runtime’s first
scheduling scan, so republishing, completion, and daemon restart do not reset it. Missing or high
load samples hold admission even after the gap expires.
`{"Fixed":0}` pauses automatic starts. Budgets constrain the whole runner:
active ticket runs across every board count, including manually started runs,
reviews, approval waits, and runs whose board no longer has a start policy.
All published budgets apply together. For example, a `Fixed(3)` policy and a
load-below-6 policy allow at most three active runs and require load below 6
for each new start. Multiple fixed or load policies use the strictest limit;
a published `Fixed(0)` pauses automatic starts on the runner. Runs
that fail, finish, cancel, or enter landing release their agent budget. Landing
groups and gates use their own exclusive landing resource and take no build
slots. There is no built-in parallel agent count.

The engine checks eligibility when work changes or finishes, and at least once
per second while idle or waiting. Scheduling snapshots are loaded once per
changed board and cached by revision; tracker-only boards without a policy are
skipped. A successful admission returns its committed state for subsequent
checks in that scan. Refused starts and queues retry after one minute, including
when a block can clear outside workflow state. It uses the board's recorded
blockers as well as the policy's dependencies; a dependency must finish successfully before its
dependent starts. A declared file path shared by two tickets holds the second
until the first finishes landing. These claims also apply across boards in the
same repository. Paths are exact repository-relative names; list every known
shared file. Undeclared file overlap is not predicted.

Republishing changes eligibility and the budget for future starts. Active runs
keep their definition, selected models and execution limits. Automatic queue
admission follows the admitted run's `auto_queue`, even after republishing a
policy without automatic starts. Removing `start_policy` restores manual starts
for waiting tickets; existing definitions without it retain their original pins.

The optional `limits` object changes execution allowances. Omitted fields retain
these defaults:

```json
{
  "limits": {
    "retries": 3,
    "back_edges": 3,
    "send_backs": 5,
    "models": 8,
    "reworks": 2,
    "attempts": 5,
    "stages": 16,
    "routes": 8,
    "group_members": 32
  }
}
```

`limits` sets the allowed maxima; the existing per-step `limit`, `max_attempts`,
`max_reworks`, group `limit` and definition `send_backs` choose the actual
allowances within them. The send-back allowance defaults to three, lowered when
its published maximum is lower. Group definitions inherit execution limits.
Structural wire-size, identity, evidence and storage safety checks still apply.
