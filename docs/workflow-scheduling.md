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
scheduling scan, so republishing, completion, and daemon restart do not reset it.
Continuous sweeps advance their clock while effects run, so a long landing gate
cannot freeze admission gaps or effect lease deadlines. Missing or high load
samples hold admission even after the gap expires.
`{"Fixed":0}` pauses automatic starts. Budgets constrain the whole runner.
Active ticket runs across every board count, including manually started runs,
reviews, approval waits, execution slot waits, and runs whose board no longer
has a start policy. When the fixed budget is full, tickets awaiting an
automatic start stay in their current column until an allowance opens. All
published budgets apply together. A `Fixed(3)` policy and a load-below-6 policy
permit a new start when fewer than three runs are active and load is below 6.
Multiple fixed or load policies use the strictest limit. Runs that fail, finish,
cancel, or enter landing release their start allowance. Landing groups and gates
use their own exclusive landing resource and take no build slots.

The board has one manual order. The engine uses it for eligible starts and
for runs competing for the next build slot on that board. New cards follow
filing order and keep their position across status changes. Move a card with
`farcooler task reorder ov-42 --before ov-33` (or `--after`), relative to any
other card on that board. On Mac, open View > Tasks by Status and drag within
a status section. On the iPad and Android boards, use the drag handle on the
card. On iPhone, use it in the Plan's Not Started or Waiting for a Builder
list. In the web client, open Backlog and To Do from the Plan and drag
within either section.

`task line KEY…`, `task line --build KEY…`, and `plan set KEY…` put those
cards first in the same order, keeping the relative order of other cards.
They also set membership of the manual line or plan. Clearing membership
keeps the board order. Manual line chips and plan positions are displayed in
board order. For engine-owned runs, `workflow get <workspace>` reports the
live slot positions. Changing the order updates these positions without
restarting a run. A run already building keeps its slot.

When upgrading an existing board, the agent line seeds the order first,
followed by the build line and then any remaining ranked plan tickets.

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
    "back_edges": 5,
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
`max_reworks` and definition `send_backs` choose the actual allowances within them.
`group_members` bounds build followers; landing batches use their own optional
`GroupDef.limit`, independently of that allowance. The send-back allowance defaults to three, lowered when
its published maximum is lower. Group definitions inherit execution limits.
Structural wire-size, identity, evidence and storage safety checks still apply.
