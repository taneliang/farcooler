# Stage 3 — say what the agent is doing, in the agent's words

## The complaint, verbatim

> it says things like "taskupdate" and "says", which is not the most user
> friendly experience. can we refine this? if there's a list of tasks or
> workflows or subagents, we should display a task progress indicator or number
> of subagents or something like that. if progress is blocked by a question, the
> question is the priority and we should display the question. The transcript
> should only be a transcript for stuff that the agent sends, other things
> should be surfaced differently

Stage 2 shipped a feed built from one primitive: `Step { verb, object }`, where
`verb` is a lowercased tool name. That makes `TaskUpdate` render as
`taskupdate`, and it makes the agent's own sentence render as `says …` — a
parser's vocabulary leaking onto a surface meant to be read at a glance from a
lock screen. Worse, it flattens three genuinely different things into one list:
what the agent SAID, what the agent DID, and where the agent IS in a plan.

This stage separates them.

## What the logs actually carry

Measured on this machine 2026-08-17, not assumed:

| signal | where | how much |
| --- | --- | --- |
| `TaskCreate` | claude `tool_use` | 21 files |
| `TaskUpdate` | claude `tool_use` | 47 files |
| `TaskList` | claude `tool_use` | 2 files |
| `pendingBackgroundAgentCount` | claude `turn_duration` | 30 files |
| `subagents/` directories | claude, per session | 26 |
| `update_plan` | codex | 1 file |

`TaskCreate` carries `subject`, `description`, and — the useful one —
**`activeForm`**, already a present-tense human phrase: `"Designing test
matrix"`, `"Identifying edge cases"`. `TaskUpdate` carries `taskId` and
`status`. So "3/7 · Designing test matrix" is not something to invent; it is
something to stop throwing away.

Subagents are spawned by the **`Agent`** tool. Each gets
`<project>/<session>/subagents/agent-<id>.meta.json`, 125 bytes, carrying
`agentType`, `description` (a 3-5 word task summary) and `toolUseId`.

**Corrected during task 2, by counting rather than reasoning.** This document
first claimed liveness was exact: a subagent runs until a `tool_result` with
its `tool_use_id` arrives. That is false for background spawns. Of 313 results,
**104 came back within seconds carrying `status: "async_launched"`**
(`isAsync: true`, plus an `outputFile`) while the agent went on working for
another hour. Only `status: "completed"` (209) is an ending. The "17 of 20"
above was this, not three genuinely outstanding agents — the original reading
was wrong in the direction that would have marked long-running background
agents as finished the moment they started.

**And the meta file turns out to be unnecessary for naming.** The plan assumed
`description` had to come from `subagents/*.meta.json`. It is in the parent
log's `Agent` tool_use input on all 315 calls, matching the meta file character
for character. So a spawn is nameable from the line that spawns it, on the tick
it happens, with no extra file read on a loop. Task 3 is therefore optional
enrichment rather than the source of truth, and is dropped from this stage.

Codex and cursor have nothing task-shaped worth reading (one file, once). They
fall back to the action line and the transcript, and that is honest rather than
a gap to paper over.

## The shape

Two zones per row, decided in the daemon exactly like the existing ladder —
one answer per surface, computed once.

**The signal line** — strictly prioritized, first match wins:

1. **The question**, when blocked. It outranks everything: a fleet where one
   agent needs an answer and the rest are churning is precisely the case this
   product exists for.
2. **Plan position** — `3/7 · Designing test matrix`, plus a subagent count
   when any are running.
3. **The current action** — `Writing fruit.txt` — for agents with no task
   list, which is codex, cursor, and most claude sessions.

**The transcript** — only what the agent SENDS. Its prose, verbatim, no verb
prefix. Tool actions leave the transcript entirely; they are the signal line's
fallback, not history.

**Subagents**, when running: count on the header, and a line each naming what
it is doing, from the meta's `description`.

## Task order

Sequential. One worktree, one git index — parallel agents sweep up each other's
staged work, which this project has already paid for once.

1. **Vocabulary.** Split `Step` into what it always was two of: `Said { text }`
   for agent prose and `Did { verb, object }` for a tool action. Add
   `TaskState { done, total, active_form }` and
   `Subagent { id, description, running }`. Keep `Vec<TurnEvent>` — a line
   carrying two facts must emit both, the lesson from stage 2's first bug.
2. **claude parsing.** `TaskCreate`/`TaskUpdate`/`TaskList` into `TaskState`;
   `Agent` tool_use and its matching tool_result into `Subagent`; `text` blocks
   at `stop_reason: end_turn` into `Said`; everything else into `Did`.
3. ~~**Subagent meta.**~~ **Dropped** — the parent log already carries every
   `description`, so this would be a file read per tick buying nothing.
4. **The fold.** Per-pane task and subagent state in `watch.rs`, surviving
   across ticks, cleared when a turn ends.

   The parser is line-at-a-time and stateless by signature, so no single line
   can state a tally: `TaskCreate` carries the phrase and **no id**,
   `TaskUpdate` carries id and status and **no totals**, and `TaskList` states
   a whole list but was asked for in 6 of 340 task-using sessions. `TaskState`
   therefore reports one task as one line stated it, and the running tally is
   assembled here. The create-to-id join is by creation order — the k-th create
   came back as id `"k"` in 289 of 289 creates on this machine — and if that
   ever breaks, the COUNT must survive and only the phrase go missing.

   `TaskUpdate` has a `deleted` status, so **a total can shrink**. A fold that
   assumes totals only grow will report `7/6`.

   **Corrected during task 4, by driving a real session rather than counting
   one.** The create-to-id join above is wrong, and wrong in a way the corpus
   count could not show: task ids are numbered **per SESSION, not per turn**. A
   second turn that creates seven tasks gets ids `"8"` through `"14"`. The "289
   of 289" holds because it measured a session's FIRST turn, which is the only
   turn in most of them. Folded as written, a seven-task list read **`3/11`,
   with no phrase at all** — an update naming an id the fold had never assigned
   both failed to join and added a task on top of the seven. Both halves of the
   row wrong, on the second thing anybody would try.

   The fix costs nothing that was being saved: the create's RESULT states the id
   outright (`toolUseResult.task.id`), one line after the create that carried
   the phrase. Task 2 read that result and deliberately dropped it — "a create's
   result is not a second task" — so the parser now emits it as the id-only
   shape of `TaskState`, and the fold joins a create to the very next create
   result rather than to a position in a turn. What is left to infer from order
   is one line apart instead of a whole turn.
5. **The signal line and the transcript**, in `feed.rs` and the ladder, with
   the priority above. Verb humanization dies here — no `says`, no
   `taskupdate`.
6. **Projections and Swift.** Every new field must reach
   `workspace_list_terminal_json`, `terminal_event_json`, the iOS projection,
   AND be decoded and rendered on the Mac. This codebase has been bitten three
   times by a field correct on one path and missing from another; once it made
   a whole feature invisible in the shipped app with every test green.

## Verification that counts

Tests did not catch a single one of the three real bugs stage 2 shipped with.
Every task ends by driving a real agent in a scratch daemon and reading the
row. For this stage specifically: run a claude session that creates a task list
and spawns subagents, and confirm the row reads `3/7 · Designing test matrix`
with the subagents named — not `taskupdate`.
