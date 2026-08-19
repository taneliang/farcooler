# What the three agents write to disk

Observed, not documented. Read across every session log on one developer's
machine in August 2026: 276 claude files, 183 codex, and all four cursor
transcripts that exist. Every claim here names what it was seen in. Anything
that could not be observed says so rather than being filled in with something
reasonable.

These are private formats with no compatibility promise. The live suite
(`cargo test -p farcooler-core --test live_agents -- --ignored`) checks the
fields below still exist; when one disappears, this document is what says what
it used to be.

## The one thing none of them do

**No agent records that it is waiting for a human.**

- claude writes the OUTCOME of a permission decision — `toolDenialKind` values
  `automode-unavailable`, `automode-blocked`, `user-rejected` — and never the
  pending state. Searched across 276 files.
- codex has nothing approval-shaped in 183 files.
- cursor shows none in its four, which is suggestive rather than conclusive.

So a log reader cannot answer "does this need me right now". That stays with the
footer-scoped screen matching from stage 1, which is the state the product
exists to surface and the one place the screen is still authoritative.

## claude

`~/.claude/projects/<slug>/<session-uuid>.jsonl`

**Slug:** every `/` and `.` becomes `-`, a leading `/` becomes a leading `-`,
and runs are **not** collapsed. `/Users/e-liang/.unixconfig` →
`-Users-e-liang--unixconfig`. Established across 35 real directories and one
clean live test. No truncation behavior was seen, but no path long enough to
provoke one was tried.

**Turn starts:** `type == "user"` carrying `promptId`, `promptSource` and
`permissionMode`. A tool result is also `type == "user"` — it is told apart by
having `message.content[].type == "tool_result"` and no `promptSource`.

`promptSource` values seen, recounted 2026-08-16 across all 766 files under
`~/.claude/projects/` (the corpus has grown since the 276 this document opens
with): `typed` 477×, `sdk` 311×, `system` 186×, `queued` 95×,
`suggestion_accepted` 2×. An earlier draft of this document listed only
`typed | queued | system`, which missed the second most common value on the
machine. **The parser checks that the key is PRESENT, not what it holds**
(`crates/core/src/session_log/claude.rs`, `turn_start`), and that is
deliberate: the list above is what one machine happened to contain in August
2026, not a closed set. Do not "fix" the parser to match it — narrowing to a
value list is how the next value claude invents becomes a turn that never
starts.

**Turn ends:** `type == "system"`, `subtype == "turn_duration"`, carrying
`durationMs` and `messageCount`, and `pendingBackgroundAgentCount` only when it
is above zero — the key is absent, not zero. The preceding assistant record has
`message.stop_reason == "end_turn"`.

**It is NOT always written**, and an earlier draft of this document said it
was. Recounted 2026-08-16 across the same 766 files: 315 carry at least one
turn start, and **243 of them contain no `turn_duration` record at all** — 77%. The
overwhelming majority are not interactive sessions: 230 have `entrypoint ==
"sdk-cli"` only and 5 `sdk-ts`, which are programmatic sessions (Far Cooler's
own ACP shim among them) that never reach the code path that writes it. But
"all of them are SDK sessions" would also be wrong: 6 are `cli` only, and 2
carry both. Across the 73 `cli`-only files that have a turn start, 752 turn
starts produced 711 `turn_duration` records — so an interactive session writes
one almost every turn, and "almost" is the whole point. **A reader must not
wait for a `turn_duration` that may never arrive.** It is not a version
difference either: files with and without it span the same CLI versions
(2.1.220 through 2.1.233).

**Claude narrates while it works too**, and it is most of what it says.
Counted 2026-08-18 across the 40 largest transcripts on this machine, by
`stop_reason` and the block types in `message.content`:

| count | `stop_reason` | blocks |
| --- | --- | --- |
| 18064 | `tool_use` | `tool_use` |
| 7025 | `tool_use` | `thinking` |
| **5623** | **`tool_use`** | **`text`** |
| 696 | `end_turn` | `text` |
| 213 | `end_turn` | `thinking` |
| 30 | `stop_sequence` | `text` |
| 4 | — | `thinking` / `text` |
| 2 | `tool_use` | `text` + `thinking` + `tool_use` |

So a `text` block is nearly always mid-turn narration — *"Fails as expected.
Implementing."*, *"Task 2 landed. Verifying, then removing the shared-file
contention before the next wave."* — written on its own `assistant` record one
line before the tool call it describes, and it outnumbers closing prose eight
to one. Prose and a tool call share a record twice in 31 thousand, so a reader
that must choose between them is choosing on the 5623 records where there is
nothing to choose.

**Other record types seen:** `assistant`, `attachment`, `ai-title`,
`agent-name`, `mode`, `permission-mode`, `last-prompt`, `queue-operation`,
`file-history-snapshot`, `file-history-delta`, `bridge-session`, `pr-link`.

**Fields worth reading:** `aiTitle` (re-emitted at every checkpoint, never
changed within a session), `permissionMode` (`auto` 2373×, `default` 92×),
`effort`, `gitBranch`, `cwd`, `sessionId`, per-message `usage`, and
`message.content[].type == "tool_use"` with `name` (`Bash`, `Edit`, `Read`,
`Write`, `Task`, `TaskCreate`).

**Task lists** are three tools, counted 2026-08-17 across the 340 files that
use them at all:

- `TaskCreate` — `input` carries `subject`, `description`, and `activeForm`, a
  present-tense phrase already written for a human ("Designing test matrix").
  255 of 289 creates carried `activeForm`; the other 34 carried only the first
  two. **The create does not carry the task's id.** The id is assigned in the
  RESULT (`toolUseResult.task.id`), and it is the creation ordinal: the k-th
  create in a session came back as `"k"` 289 times out of 289, no exceptions.

  **In a SESSION, which is not a turn** — and that distinction cost a row.
  Driven live on 2026-08-17, a second turn creating seven tasks got ids `"8"`
  through `"14"`; claude's own task panel counted 21 across three such turns.
  So anything that keeps a per-TURN list (a row shows the plan of the turn it
  is watching, not of the session) cannot number that list by counting the
  creates it has seen: the read the daemon does is the RESULT's id, which is
  stated outright one line after the create that carried the phrase.
- `TaskUpdate` — `input` carries `taskId` and `status`. Statuses seen:
  `completed` 251×, `in_progress` 214×, `pending` 1×, `deleted` 1×, and one
  update with no status at all (it changed a description).
- `TaskList` — empty `input`; the result carries the whole list as
  `toolUseResult.tasks[]`, each `{id, subject, status, blockedBy}` with no
  `activeForm`. Asked for in 6 of the 340 sessions.

**So no single line states a tally.** `done`/`total` can only be folded across
lines, which is why `TurnEvent::TaskState` reports one task rather than a
count, and why the running tally lives in the daemon.

**Subagents are separate files**, not a flag. Each subagent is
`<project>/<parent-session-uuid>/subagents/agent-<id>.jsonl` with a sibling
`.meta.json` carrying `agentType`, `description`, `spawnDepth`, sometimes
`model`, and `toolUseId`.

What the PARENT log carries about them, counted the same day across 315 spawns:

- The spawning `tool_use` is `name == "Agent"`, and its `input.description` is
  the same 3-5 word summary as the meta file's — identical in all 315, so a
  spawn can be named from the parent log alone, on the line it happens.
- The matching `tool_result` is recognizable without knowing the tool name:
  its `toolUseResult` carries `agentId`, which no other tool's result did
  (313 of 313).
- **A result arriving does NOT mean the subagent finished.** A background
  spawn (`input.run_in_background: true`) gets its result back within
  seconds, carrying `status: "async_launched"`, `isAsync: true` and an
  `outputFile` to read later, while the agent runs on. 104 of the 313 came
  back that way; only `status: "completed"` (209) is an ending.
- Two of the 315 results carry a plain string `toolUseResult` (a tool error)
  with no `agentId`. Those are invisible to a line-at-a-time reader: nothing
  in them says which tool they came from.

`isSidechain` exists and is a trap. It appears 40,724 times in one project's
logs and is `false` in every one of them — `true` was never observed. So the
field is present, always says no, and marks nothing. An earlier draft of this
document said it "appears zero times anywhere", which was wrong in a way that
matters: a reader could conclude the field had been removed and that claude had
changed its format. It is there. It is simply never set.

**Hazards.**

- `timestamp` is not monotonic. 235 of 276 files have adjacent lines whose
  timestamps go backwards. **Line order is the only order to trust.**
- Single lines exceed 1 MB (largest seen 1.35 MB) in files over 100 MB.
- Schema drifts across CLI versions; this machine spans 2.1.44 to 2.1.233, and
  `ai-title` appears in only 98 of 276 files.
- `cwd` can change within one session when the agent moves to a worktree.

## codex

`~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`

The directory is named for the local date the session STARTED. Files do not
rotate; a long session keeps appending to the file it opened.

**Turn starts:** `event_msg` with `payload.type == "task_started"`, carrying
`turn_id`, `started_at`, `model_context_window`.

**Turn ends:** `payload.type == "task_complete"` with a matching `turn_id`,
plus `completed_at`, `duration_ms`, `time_to_first_token_ms` and
`last_agent_message`. A turn can instead end with `turn_aborted`, carrying
`reason`.

**Units, and this is a trap:** `started_at` and `completed_at` are unix
**seconds**; `duration_ms` and `time_to_first_token_ms` are **milliseconds** —
in the same payload. And `session_meta` calls the context window
`context_window` while `task_started` calls it `model_context_window`.

**Other payloads:** `token_count` (`total_token_usage`, `rate_limits`),
`agent_message` (`phase: final_answer`), `user_message`, `session_meta`
(`cwd`, `originator`, `cli_version`), `turn_context` (`approval_policy`).

**What happened during a turn — and this changed under us.** Everything above
was read from a corpus that predates codex 0.147.0. That version stopped
writing `event_msg`/`agent_message` and now reports each step as
`event_msg`/`item_completed`, carrying `turn_id`, `started_at_ms`,
`completed_at_ms`, and an `item` whose own `type` says what happened:

| `item.type` | what it carries |
| --- | --- |
| `UserMessage` | `content[].text` — the prompt that opened the turn |
| `Reasoning` | `summary_text`, `raw_content` — both empty in every record seen |
| `AgentMessage` | `content[].text` plus `phase`, either `commentary` or `final_answer` |
| `FileChange` | `changes`, an absolute path mapped to `{type: add, content}` |
| `CommandExecution` | `command` as argv, `process_id`, `cwd`, `status` |

The two shapes do not overlap. Of 264 rollouts on the machine this was read
from, 251 carry `agent_message` and 10 carry `item_completed` — the 10 being
the most recent — and **not one file carries both**. A reader that knows only
`agent_message` therefore shows nothing at all on current codex while its turn
clock keeps working perfectly, which is a failure with no symptom. Read both:
old rollouts stay on disk, and a user can downgrade.

`session_meta` flattened at the same time: its fields sit directly on
`payload` in 0.147.0 where older rollouts nest them under `payload.payload`.

**Codex narrates while it works, in BOTH shapes.** Counted 2026-08-18 across
324 rollouts: `agent_message` splits 901 `final_answer` to 498 `commentary`,
and `item_completed`/`AgentMessage` splits 32 to 161. Commentary is a whole
sentence about what the agent is about to do — *"I'm using the `ios-fix`
workflow because this is a live-device iOS layout bug: I'll inspect the current
phone state…"* — written as the turn goes, which is the only prose that exists
while anyone is watching. The agent's private thinking is **not** this: codex
writes that as its own `agent_reasoning` payload, 896 records, which the parser
does not read.

`session_meta.originator` is `codex-tui` for a real interactive session and
`farcooler`/`vscode` for a programmatic one — useful for ignoring panes that are
not a person's terminal.

**Hazards.**

- 5 of 183 files end on a `task_started` with no `task_complete` or
  `turn_aborted` anywhere. A turn can stay open forever; a reader must not wait
  for a completion that will never come.
- `rate_limits` is plan-dependent: a `plus` plan carries a `credits` object and
  can have `secondary: null`; a `team` plan differs.
- Largest file seen is 60 MB across 3,593 lines.

## cursor

`~/.cursor/projects/<slug>/agent-transcripts/<uuid>/<uuid>.jsonl`

**Slug, recovered from cursor's own shipped source** (`workspace-paths.js`):
every non-alphanumeric character becomes `-`, runs **collapse** to one, leading
and trailing hyphens are trimmed. If the joined path exceeds 92 characters it is
cut to 84 and given `-` plus the first seven hex characters of the full path's
sha256. Note this differs from claude, which does not collapse.

**Records** are either `{role, message}` or `{type, status}`.

**Turn starts:** `role == "user"`, whose text is wrapped as
`<timestamp>…</timestamp><user_query>…</user_query>`. The timestamp is a
localized human string — `Sunday, Aug 16, 2026, 1:19 AM (UTC-7)` — not ISO or
epoch, and needs its own parser if it is ever wanted.

**Turn ends:** `{"type":"turn_ended","status":"success"}`. Only `success` and
`error` were ever seen.

**Tool calls:** `message.content[]` blocks of `type == "tool_use"`. The only
`name` ever observed is `Shell`, whose `input` carries both `command` and a
human-written `description`.

**Hazards, and this section is thin on purpose.**

- Only four transcript files exist on this machine, together under 2 KB. Most
  per-project cursor directories have no `agent-transcripts` at all.
- No tool-result record exists in any sample — nothing carries a command's
  output.
- Only one tool name was ever seen, so the `{command, description}` input shape
  is confirmed for `Shell` and for nothing else.
- Every sample shut down cleanly, so nothing here says what a truncated or
  interrupted transcript looks like.

Anything built on cursor's log is built on much less evidence than claude's or
codex's, and should fail soft rather than assume.

## Which file belongs to which pane

**codex — `lsof`.** The process holds its rollout file open, so
`lsof -p <foreground pid>` names it. Two nuances: the file is not opened until
the FIRST TURN IS SUBMITTED, so a freshly started pane has nothing to find; and
a codex started through a wrapper holds a second, unrelated jsonl open (the
wrapper's own shadow log), so match `~/.codex/sessions/**/rollout-*.jsonl`
specifically rather than "an open jsonl".

**claude and cursor — slug, then title.** They append and close, so `lsof` finds
nothing. The pane's cwd gives the project directory by the slug rules above, and
within it the file being appended to is the live session. Two panes in one cwd
are told apart by the pane's OSC title: claude's `aiTitle` equals the title
exactly once the leading activity glyph is stripped. Verified live — pane title
`✳ Write haiku about lighthouse`, transcript
`"aiTitle":"Write haiku about lighthouse"`.

**And a file one pane holds is not another's.** The title only tells panes
apart once BOTH have written a file to compare. Before that — a second claude
started in a workspace where the first is already running — the running pane's
session is the only candidate in the directory, and a rule that takes the lone
candidate hands it straight over: both rows then show one conversation's
summary, plan position and actions. So a file some other pane is already
tailing is excluded before the count is taken, which leaves the new pane with
nothing until it writes its own. That is the honest answer, and the same
direction everything else here errs in.

It has to be, because the mistake is not self-correcting: a pane only
re-derives its join when it holds no file, or when `join_looks_dead` fires —
and that wants the pane to look busy AND its file to have been silent for 30s,
which a file its real owner is actively writing never is.

`terminals.agent_session_id` already exists in the store, unpopulated, for
exactly this.

**A trap when testing by hand:** a `claude` spawned from inside a claude session
inherits `CLAUDE_CODE_SESSION_ID` and adopts the PARENT session's `aiTitle`,
which makes the join look broken when it is not. Strip `CLAUDE_CODE_*` and
`CLAUDECODE` first.
