# Real session-log excerpts

## Identifiers are synthetic

Every uuid here is fake. Real session, message, tool-use and turn ids were
replaced with `00000000-0000-4000-8000-0000000000NN`, consistently — the same
source id always became the same replacement, so a `parentUuid` still points at
the record it pointed at before. A few point outside their excerpt, which is
simply what an excerpt of a longer session looks like. The macOS per-user temp
directory name was replaced too, because it identifies a machine rather than a
format.

Scrub anything added here the same way. These fixtures exist to pin the SHAPE of
a record; no value in them needs to be real, and an obviously fake id is one
nobody can later mistake for a real one.

Every parser written against `docs/agent-session-logs.md` should be testable
against a file in here. That reference document was built by reading real
logs on one developer's machine in August 2026; these fixtures are short,
redacted cuts of the same real files, kept small enough to read on one
screen. Nothing here was invented — each fixture is real JSONL lines, with
paths and message text cut down, not synthesized records.

## claude

| fixture | shape | source version |
| --- | --- | --- |
| `claude-complete-turn.jsonl` | a complete turn | 2.1.233 |
| `claude-out-of-order-timestamps.jsonl` | adjacent lines whose `timestamp` goes backwards | 2.1.220 |
| `claude-task-list.jsonl` | a task list being created, worked and listed | 2.1.227 |
| `claude-subagents.jsonl` | one subagent spawned and finished, one spawned in the background | 2.1.223 |

**`claude-complete-turn.jsonl`** (7 lines) is drawn from a real session under
`~/.claude/projects/`, a `Write a haiku... then tell me what 'esc to
interrupt' means` prompt. It has the turn-opening `user` record
(`promptId`/`promptSource: "typed"`), two `assistant` records that make up
one logical model turn split across lines (`thinking` then `tool_use` for
`Write`, both `stop_reason: "tool_use"` and the same message `id`), the
matching `tool_result`, the closing `assistant` record with `stop_reason:
"end_turn"`, a `system`/`stop_hook_summary`, and the closing
`system`/`turn_duration`. The file it came from had 25 lines total; the
`attachment`, `mode`, `ai-title`, and `last-prompt` records between them were
dropped as noise, not redacted — nothing in the kept lines was altered except
what's listed below.

Redacted: the real path (a worktree scratchpad whose name encoded
`/Users/e-liang/...`) is replaced everywhere with `/Users/example/project`.
The final assistant reply's ~1,500-character prose explanation of `esc to
interrupt` is cut to one sentence — record shape needs `stop_reason` and a
`text` block, not the essay. The opaque `thinking` block's base64 `signature`
field is replaced with a placeholder (it's meaningless to a parser and was
several hundred bytes). The local stop-hook's shell command line is replaced
with a generic equivalent, since it named a machine-local `$SUPERSET_HOME_DIR`
path.

**`claude-out-of-order-timestamps.jsonl`** (5 lines) is a real backward jump:
line 2 (`hook_success` attachment) carries timestamp `22:50:49.010Z`, one
second *before* line 1's `queue-operation` at `22:51:24.706Z` — 35 seconds
backwards, confirming the reference doc's warning that `timestamp` is not
monotonic and line order is the only order to trust. This machine has 235 of
276 files with this property; this was the first one a scan found.

Redacted: the real `cwd` (`/Users/e-liang/Library/Application
Support/com.overnight.Overnight/worktrees/overnight-test`) is replaced with
`/Users/example/project` everywhere it appears. The `SessionStart` hook's
`stdout` and `hookAdditionalContext` fields held a multi-kilobyte dump of
skill instructions unrelated to timestamp ordering; both are replaced with a
short placeholder. The opening user message's text (a question about domain
names, unrelated to what this fixture demonstrates) is replaced with a
placeholder.

**`claude-task-list.jsonl`** (9 lines) is what a row shows instead of
`taskupdate`. Drawn from a real `sdk-cli` session that was asked to create a
task list and work through it, it has: a `TaskCreate` (`activeForm: "Designing
test matrix"`) and its result, which is where claude states the new task's id
(`toolUseResult.task.id: "1"`); a second `TaskCreate` and its result; a
`TaskUpdate` to `in_progress` and its result; a `TaskUpdate` to `completed`;
and a `TaskList` with the result that carries the whole list under
`toolUseResult.tasks[]`. The source file had 155 lines. The `TaskList` result
names five tasks where this excerpt keeps only the first two creates — that is
simply what a cut out of a longer session looks like, and it is also the point:
the list result is the only record that states a whole list.

Redacted: every uuid (`parentUuid`, `uuid`, `promptId`, `sessionId`,
`sourceToolAssistantUUID`) is replaced with a synthetic one, consistently. The
real `cwd` (a `com.farcooler.FarCooler` worktree under the user's Library) is
replaced with `/Users/example/project`. Nothing else was altered; the tool
inputs, results and `toolUseResult` bodies are as claude wrote them.

**`claude-subagents.jsonl`** (4 lines) is a spawn that ends and a spawn that
does not. Line 1 is an `Agent` `tool_use` with `run_in_background: false`,
line 2 is its `tool_result` 22 minutes later with `toolUseResult.status:
"completed"`. Line 3 is a second `Agent` `tool_use` with
`run_in_background: true`, and line 4 is its `tool_result` **1.9 seconds
later**, `status: "async_launched"` — the agent is still running, and a reader
that treats any result as an ending calls it finished the instant it started.
Both results carry `toolUseResult.agentId`, which is how a line-at-a-time
parser recognizes them without knowing which tool they came back from.

Redacted: every uuid is replaced with a synthetic one, and both `agentId`s
(`a000000000000000a`, `a000000000000000b`) with obviously fake hex of the same
shape, including inside the result text and the `outputFile` path. The real
worktree paths are replaced with `/Users/example/project`. The two agent
prompts (several KB each, naming private files) and the returned report text
are cut to a placeholder — the record shape needs the keys, not the essay. The
`usage` and `toolStats` blobs (token accounting no parser here reads, ~2 KB of
numbers) are dropped from `toolUseResult`, the same way noise records are
dropped from the other fixtures.

## codex

| fixture | shape | source `cli_version` |
| --- | --- | --- |
| `codex-complete-turn.jsonl` | a complete turn | 0.139.0 |
| `codex-unmatched-task-started.jsonl` | `task_started` with no completion anywhere in the file | 0.147.0 |
| `codex-item-completed-turn.jsonl` | a complete turn in the shape 0.147.0 writes | 0.147.0 |

**`codex-complete-turn.jsonl`** (11 lines) is a real "say hi" session from
`~/.codex/sessions/`, `session_meta.originator: "codex_exec"`,
`plan_type: "team"`. It has `session_meta`, `event_msg/task_started`
(`turn_id: "019ec775-7118-7ec0-8301-0152c3fe920f"`), the `turn_context` and
`response_item` records that open a turn, `event_msg/user_message`,
`event_msg/agent_message` (`phase: "final_answer"`), `event_msg/token_count`
(with the plan's `rate_limits` shape — `credits: null` for a `team` plan),
and the matching `event_msg/task_complete` carrying the same `turn_id`. Every
line is unaltered from the original except for the two redactions below —
this is the smallest complete-turn file found on this machine (11 lines
total, nothing was dropped).

Redacted: `session_meta.payload.base_instructions.text` (the ~21 KB system
prompt Codex ships with every session — boilerplate, not something a log
parser reads) is replaced with a placeholder, as is the developer message
carrying the sandbox/apps/skills preamble. `session_meta.payload.git` named a
private repository and GitHub username; its values are replaced with
`"REDACTED"`. `cwd` (`/Users/e-liang/Dev/Verdela`) is replaced with
`/Users/example/project` everywhere it appears (`session_meta.payload.cwd`,
the `<environment_context>` block, and `turn_context.cwd` /
`workspace_roots`).

**`codex-unmatched-task-started.jsonl`** (2 lines) is a session that opens
with `session_meta` and `event_msg/task_started`
(`turn_id: "019fea6f-930c-7222-b619-dfb96a6fb2ed"`) and then the file simply
ends — no `task_complete`, no `turn_aborted`, anywhere. This is the whole
file; nothing was trimmed. The reference doc says 5 of 183 files on this
machine end this way; this is the smallest of them. Its `originator` is
`"farcooler-probe"` (a programmatic probe session, not an interactive
`codex-tui` one) — noted here since the reference doc calls out `originator`
as the field for telling those apart, and this fixture is not an example of
a real person's terminal going quiet mid-turn, just of the record shape.

Redacted: `session_meta.payload.base_instructions.text` (~7 KB of system
prompt boilerplate) is replaced with a placeholder. `cwd` was already a
`/var/folders` temp path, not a home directory, so nothing needed replacing
there.

**`codex-item-completed-turn.jsonl`** (9 lines) is a real turn captured by
driving codex 0.147.0 in a pane — "create a file called fruit.txt containing
the word banana, then say what you did" — after that turn came back with a
correct status, a correct clock, and a completely empty feed. It is the
evidence for why: this version writes no `event_msg`/`agent_message` at all.
It has `session_meta`, `event_msg/task_started`, one `event_msg/item_completed`
for each distinct `item.type` the turn produced (`UserMessage`, `Reasoning`,
`AgentMessage` at `phase: commentary`, `FileChange`, `CommandExecution`,
`AgentMessage` at `phase: final_answer`), and the closing
`event_msg/task_complete`. The source rollout had 32 lines; repeats of an
`item.type` already present were dropped, along with the `response_item`,
`turn_context`, `world_state` and `token_count` lines, as noise — nothing in
the kept lines was altered except the redactions below.

Note `session_meta`'s fields sit directly on `payload` here, where
`codex-complete-turn.jsonl` (0.139.0) nests them under `payload.payload`. That
difference is real and unedited, and it is why the two fixtures cannot share
one reader.

Redacted: `base_instructions` (the system-prompt boilerplate) is replaced with
a placeholder and `git` with `"REDACTED"`. The real worktree path is replaced
with `/Users/example/project` everywhere it appears, including inside
`FileChange.changes` keys and `CommandExecution.cwd`. Every uuid is replaced
consistently, so a `turn_id` still matches the `task_started` it belongs to,
and codex's opaque model item ids (`msg_…`, `rs_…`) are cut to a stub — no
parser reads them and they are account-scoped.

## cursor

Cursor's coverage is thinner than claude's or codex's by necessity, not by
neglect: only four `agent-transcripts/*/*.jsonl` files exist on this
machine, totaling 1,871 bytes — under 2 KB combined. Both fixtures below are
drawn from that full population; there was no larger pool to choose a small
excerpt from.

Cursor transcripts carry no version field anywhere — no `session_meta`
equivalent, no sibling metadata file next to the transcript. The only
version number known for this install is `cursor-agent 2026.08.11-e8db854`,
recorded in `crates/core/captures/README.md` from the shipped binary itself,
not from anything the transcript writes.

| fixture | shape |
| --- | --- |
| `cursor-complete-turn.jsonl` | a complete turn |
| `cursor-turn-ended-error.jsonl` | `turn_ended` with `status: "error"` |

**`cursor-complete-turn.jsonl`** (4 lines) is one full transcript, unedited
except for the note below: the turn-opening `user` record (message text
wrapped in `<timestamp>...</timestamp><user_query>...</user_query>`), an
`assistant` record with a `tool_use` block (`name: "Shell"`, `input` carrying
`command` and `description`), a closing `assistant` text record, and
`{"type":"turn_ended","status":"success"}`.

Redacted: nothing needed redacting. The transcript's only content is a
synthetic-looking probe command (`echo banana > fruit.txt`) with no home
path or credential-shaped text in it.

**`cursor-turn-ended-error.jsonl`** (1 line) is the entire content of one of
the four transcript files: `{"type":"turn_ended","status":"error","error":
"Named models unavailable Free plans can only use Auto..."}`. The reference
doc's hazards section says only `success` and `error` were ever seen for
`turn_ended.status`, and flagged this shape as one that might not exist to
capture — it does exist, in one of the four files, and this is it, unedited.

Redacted: nothing. The line is the whole file, and it contains no path or
message text beyond the plan-limit error string itself.
