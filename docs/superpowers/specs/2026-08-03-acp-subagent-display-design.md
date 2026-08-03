# Design: subagent transcripts render as subagent transcripts

Date: 2026-08-03
Status: APPROVED (design); implementation not started
Extends `docs/superpowers/specs/2026-08-01-native-agent-view-design.md`.

Scoped to subagents dispatched through the `Task` tool, which ACP delivers in
full. Claude Code *workflows* are a different mechanism that ACP does not
deliver at all — see "Workflows are not in scope, and why" at the end. They get
their own spec.

## Problem

A subagent's work is attributed to the agent that dispatched it.

`crates/agent/src/session.rs:225` opts into nested subagent transcripts at
`initialize`. Nothing downstream reads the result. `acp/wire.rs` models no
`_meta` field, so serde discards the only thing that carries the structure, and
every frame a subagent produces is flattened into the parent's stream as though
the parent had produced it.

Replaying a live capture through the current normalizer (captured 2026-08-03,
`crates/agent/tests/fixtures/subagent_with_cap.jsonl`) yields:

```
AGENT  "I'll dispatch the subagent."
TOOL   "Task" (think)
TOOL^  "Count lines in main.rs"
AGENT  "I'll read the file."          <- the SUBAGENT
TOOL   "Read main.rs" (read)          <- the SUBAGENT
AGENT  "3 lines. File path: ..."      <- the SUBAGENT
AGENT  "The subagent reports `main.rs` has **3 lines**."
```

A reader concludes the top-level agent read the file itself and then narrated
its own result back to itself. The `Read` renders as a sibling of the `Task`
row that caused it.

**Zero gaps are produced.** Every frame is a `sessionUpdate` kind we already
model, so nothing trips the "history is missing" honesty check. The transcript
is wrong and says it is fine — the exact failure mode `normalize.rs` was
written to prevent, arriving through a door nobody had modelled.

This matters in proportion to how many subagents a session dispatches, and
sessions here routinely dispatch many.

## What the wire actually carries

Established by two live captures against `@agentclientprotocol/claude-agent-acp`
(same prompt, capability on and off; both committed as fixtures).

There are no nested sessions and no new update kinds. Every frame arrives as an
ordinary top-level `session/update` on the **same `sessionId`**, and all the
structure lives in `_meta.claudeCode`:

- **The dispatch** — a normal `tool_call`, `title: "Task"`, `kind: "think"`,
  tagged `subagent: true`.
- **The subagent's work** — normal frames tagged
  `parentToolUseId: "<the dispatch's toolCallId>"`.
- **The result** — on the dispatch's final `tool_call_update`, under
  `_meta.claudeCode.toolResponse`, fully structured: `agentType`,
  `resolvedModel`, `totalTokens`, `totalToolUseCount`, `totalDurationMs`,
  `status`, and a `toolStats` breakdown. No text parsing needed.

What the `subagent-transcript` capability changes, measured:

| | capability on | capability off |
|---|---|---|
| dispatch `tool_call` + updates | yes | yes |
| subagent's **tool calls** (`parentToolUseId`) | yes | **yes** |
| subagent's **messages** | yes | **absent** |

Two consequences. The flag gates the subagent's *narration*, not its tool
calls — so the mis-nesting of subagent tool rows exists whether or not we opt
in, and turning the flag off would reduce the damage without fixing it. And the
flag stays **on**: without it we lose the narration entirely, and with
attribution fixed that narration is worth having.

Single capture per arm, so treat the table as strong evidence rather than a
settled contract.

## What we are building

`_meta.claudeCode.parentToolUseId` is a parent pointer. We carry it through to
the client and render the tree it describes.

### Wire layer — `crates/agent/src/acp/wire.rs`

One new struct, added to `AgentMessageChunk`, `AgentThoughtChunk`, `ToolCall`,
and `ToolCallUpdate`:

```rust
/// Structure the adapter carries out-of-band, on `_meta.claudeCode`.
#[derive(Debug, Default, Deserialize)]
pub struct ClaudeMeta {
    /// This tool call IS a subagent dispatch.
    #[serde(default)]
    pub subagent: bool,
    /// This frame is a subagent's work; the value is the dispatching call's id.
    #[serde(rename = "parentToolUseId", default)]
    pub parent_tool_use_id: Option<String>,
    /// Present on the dispatch's final update.
    #[serde(rename = "toolResponse", default)]
    pub tool_response: Option<SubagentResult>,
}
```

Lenient exactly as the rest of the file is: an absent `_meta` deserializes to
the default, and a `claudeCode` block that grows fields we do not model is
ignored rather than fatal.

### Event layer — `crates/agent/src/event.rs`

`Message`, `ToolCall`, and `ToolUpdate` each gain `parent: Option<String>`.
`ToolCall` gains `subagent: bool`; `ToolUpdate` gains
`subagent: Option<SubagentSummary>`, populated on completion from
`toolResponse`.

Two names for two layers, deliberately: `SubagentResult` is the wire shape
`wire.rs` deserializes, `SubagentSummary` is the normalized shape clients
render. The wire's field set is the adapter's to change; ours is not.

All `#[serde(default)]`. Events already in SQLite decode unchanged — an absent
parent means top-level, which is what they were.

**No protobuf change.** `AgentEvent` crosses the wire as `payload_json`
(`proto/farcooler.proto:247-257`), deliberately, so that one Rust definition
stays the single source of truth. Adding fields to the enum is the whole
protocol change.

### Transcript layer — `apps/shared/AgentKit/Sources/AgentKit/Transcript.swift`

`TranscriptRow.Kind` gains a `.subagent` case wrapping the existing `ToolRow`
plus `children: [TranscriptRow]` and an optional summary.

`apply(_ event:)` routes any event carrying a `parent` into that parent's
`children` instead of `rows`, through a `toolCallId → row index` map. Six
subagents interleaving in one stream become six independent lookups: no
ordering assumptions, no nesting state machine to drift.

Shared, so the phone and the Mac cannot disagree about one session — the
property the whole derivation model exists to hold.

### Rendering

A block auto-expands while `inProgress` and collapses on completion. Collapsed,
it is a single row: title plus the summary line (agent type, tool count,
tokens, duration, outcome). Expanded, it shows its **last 3 children** with a
`… N more` affordance, so a block's height is bounded whether it holds three
rows or three hundred. The cap applies to every expanded block, automatic or
manual; `… N more` opens the block's full history.

One rule on top: **a manual toggle wins permanently.** Once a user opens or
closes a block, automatic collapse never moves it again. Without this, a block
closes itself under someone who is mid-read.

The mis-attribution bug dies as a structural consequence rather than as a fix:
a `Message` carrying a parent has nowhere to render except inside that parent's
block.

## Error handling

**Orphan frames.** A `parentToolUseId` naming a tool call we never saw — the
ring trimmed it, or a reconnect replayed only part of the turn. The row renders
**top-level, with a diagnostic printed**, and does *not* become a `Gap`. A gap
means content was lost; here only the nesting is unknown, and the content is
all present. Emitting one gap per orphan frame would also turn a single missing
parent into dozens of false "history missing" breaks — the bug the
`AvailableCommandsUpdate` arm already exists to prevent.

**Epoch reset.** `resetForNewEpoch()` clears the `toolCallId → row` map with
`rows`. A stale map nests a new run's frames under a dead run's rows.

**Cancelled turns.** A subagent in flight when a turn is cancelled never
receives its completion update. On `TurnEnded`, any block still `inProgress`
resolves to an explicit *interrupted* state. A subagent whose outcome we do not
know must never render as one that succeeded.

## Testing

Fixture replay, following `crates/agent/tests/normalize_fixtures.rs`: real
captured frames, not hand-written samples, so a passing suite means the
normalizer survived contact with the wire.

- a subagent's message never renders as a top-level agent message — the direct
  regression test for the bug above
- the dispatch row is marked as a dispatch, and its summary carries
  `agentType`, tokens, and duration
- an orphan frame renders top-level and produces **zero** gaps
- the without-cap capture still nests tool calls, pinning the finding that
  nesting does not depend on the capability
- `TranscriptTests`: two parents with interleaved children land in the right
  blocks
- a cancelled turn leaves no subagent showing a completion mark

The throwaway `crates/agent/tests/spike_subagent.rs` is deleted once these
exist.

## Open question

Neither capture triggered a `session/request_permission` from inside a
subagent, so whether that request carries parent attribution is unknown. If it
does not, a permission prompt raised during several parallel subagents cannot
say which one is asking. Close this with a targeted capture during
implementation rather than by guessing at it now.

## Workflows are not in scope, and why

A Claude Code workflow is not a bigger subagent. Captured 2026-08-03
(`crates/agent/tests/fixtures/workflow_dispatch.jsonl`), the `Workflow` tool
call carries **no** `subagent` flag, and its internal agents emit **zero** ACP
frames. The tool response is:

```json
{"status": "async_launched", "taskId": "wn2c01m07", "runId": "wf_851d567e-f59",
 "workflowName": "alpha-beta",
 "transcriptDir": "~/.claude/projects/…/subagents/workflows/wf_851d567e-f59"}
```

The turn then ends immediately — the fleet outlives it, and its completion
arrives in a later turn.

So a workflow is visible over ACP as one opaque row and a path on disk.
Displaying its internals means tailing `transcriptDir` off the filesystem,
correlating `runId` back to a tool row, and handling a progress stream whose
lifetime is not the turn's. That is a different subsystem with different
failure modes, and folding it in here would couple two mechanisms that break in
unrelated ways.

Until that spec exists, a workflow renders as what it is: one row showing its
name and status.

## The next project

Workflow transcripts: a filesystem reader for `transcriptDir`, keyed by
`runId`, feeding the same nested-block rendering this design builds.
