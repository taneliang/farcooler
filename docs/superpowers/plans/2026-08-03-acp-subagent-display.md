# ACP Subagent Display Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Render a subagent's work inside a collapsible block belonging to the `Task` call that dispatched it, instead of flattening it into the parent agent's transcript.

**Architecture:** The ACP adapter already carries the structure on `_meta.claudeCode` — `subagent: true` marks a dispatch, `parentToolUseId` marks a subagent's frames. `wire.rs` deserializes it, `normalize.rs` puts it on `AgentEvent`, and `Transcript.swift` routes any event with a parent into that parent's block instead of the top level. Both apps then render one new `TranscriptRow.Kind` case.

**Tech Stack:** Rust (serde, tokio), Swift 6 (swift-testing), SwiftUI.

## Global Constraints

- Spec: `docs/superpowers/specs/2026-08-03-acp-subagent-display-design.md`.
- **No protobuf change.** `AgentEvent` crosses the wire as `payload_json` (`proto/farcooler.proto:247-257`).
- **Backward compatible.** Every new Rust field is `#[serde(default)]` and skipped when empty; every new Swift payload property is optional. Events already in SQLite must decode unchanged.
- **Never silently drop.** An orphan parent renders top-level with a diagnostic — never a `Gap`, never a discarded row.
- Rust tests: `cargo test -p farcooler-agent` (needs `PATH="$HOME/.cargo/bin:$PATH"`).
- Swift tests: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test --package-path apps/shared/AgentKit`. The `DEVELOPER_DIR` is required locally — the default toolchain is CommandLineTools, which has no `Testing` module.
- Swift test style: swift-testing (`import Testing`), free `@Test func` with full-sentence names, `#expect`, `Issue.record` on wrong-case. **Every test opens with a prose comment naming the real failure it prevents** — this is a hard house convention.
- Rust comment style: explain why, cite the failure that motivated the code. Match the density in `normalize.rs`.

---

### Task 1: Wire layer reads `_meta.claudeCode`

**Files:**
- Modify: `crates/agent/src/acp/wire.rs`

**Interfaces:**
- Produces: `wire::Meta` (field `claude_code: ClaudeMeta`), `wire::ClaudeMeta { subagent: bool, parent_tool_use_id: Option<String>, tool_response: Option<SubagentResult> }`, `wire::SubagentResult { agent_type, resolved_model, total_tokens, total_tool_use_count, total_duration_ms, status }`. Each of `AgentMessageChunk`, `AgentThoughtChunk`, `UserMessageChunk`, `ToolCall`, `ToolCallUpdate` gains a field `meta: Meta`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/agent/src/acp/wire.rs`:

```rust
#[cfg(test)]
mod meta_tests {
    use super::*;

    #[test]
    fn a_dispatch_is_recognized_as_one() {
        // The Task row and an ordinary tool row are the same `tool_call` on
        // the wire; `subagent` is the only thing telling them apart, and
        // without it a subagent's block has no row to hang from.
        let raw = r#"{"sessionUpdate":"tool_call","toolCallId":"t1","title":"Task",
            "_meta":{"claudeCode":{"toolName":"Agent","subagent":true}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let SessionUpdate::ToolCall { meta, .. } = update else { panic!("expected a tool call") };
        assert!(meta.claude_code.subagent);
    }

    #[test]
    fn a_subagents_frame_names_the_call_that_dispatched_it() {
        // The whole design rests on this pointer. Discarding it is what
        // attributed a subagent's words to the agent that dispatched it.
        let raw = r#"{"sessionUpdate":"agent_message_chunk","content":{"text":"hi"},
            "_meta":{"claudeCode":{"parentToolUseId":"toolu_01Wnr"}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let SessionUpdate::AgentMessageChunk { meta, .. } = update else { panic!("expected a chunk") };
        assert_eq!(meta.claude_code.parent_tool_use_id.as_deref(), Some("toolu_01Wnr"));
    }

    #[test]
    fn a_finished_subagent_reports_what_it_cost() {
        // Captured verbatim from a live adapter. These are the fields the
        // collapsed summary line is built from; parsing them out of the
        // `rawOutput` text blob instead would break the first time the
        // adapter reworded it.
        let raw = r#"{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed",
            "_meta":{"claudeCode":{"toolResponse":{"status":"completed","agentType":"general-purpose",
            "resolvedModel":"claude-opus-5[1m]","totalDurationMs":4962,"totalTokens":12479,
            "totalToolUseCount":1}}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let SessionUpdate::ToolCallUpdate { meta, .. } = update else { panic!("expected an update") };
        let result = meta.claude_code.tool_response.expect("a result");
        assert_eq!(result.agent_type, "general-purpose");
        assert_eq!(result.total_tokens, 12479);
        assert_eq!(result.total_tool_use_count, 1);
        assert_eq!(result.total_duration_ms, 4962);
        assert_eq!(result.resolved_model, "claude-opus-5[1m]");
    }

    #[test]
    fn a_frame_with_no_meta_is_still_a_frame() {
        // Most frames carry no `_meta` at all. Requiring it would fail every
        // ordinary turn.
        let raw = r#"{"sessionUpdate":"agent_message_chunk","content":{"text":"hi"}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let SessionUpdate::AgentMessageChunk { meta, .. } = update else { panic!("expected a chunk") };
        assert!(!meta.claude_code.subagent);
        assert!(meta.claude_code.parent_tool_use_id.is_none());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent meta_tests`
Expected: FAIL to compile — no field `meta` on these variants, no type `Meta`.

- [ ] **Step 3: Add the types**

Insert into `crates/agent/src/acp/wire.rs` above `pub enum SessionUpdate`:

```rust
/// The envelope the adapter hangs its own extensions from.
///
/// Everything Far Cooler needs about subagents arrives here rather than in a
/// modelled field, because ACP has no subagent concept — the adapter carries
/// the structure out-of-band and the protocol stays unaware of it.
#[derive(Debug, Default, Deserialize)]
pub struct Meta {
    #[serde(rename = "claudeCode", default)]
    pub claude_code: ClaudeMeta,
}

#[derive(Debug, Default, Deserialize)]
pub struct ClaudeMeta {
    /// This tool call IS a subagent dispatch — the `Task` row.
    #[serde(default)]
    pub subagent: bool,
    /// This frame is a subagent's work. The value is the dispatching call's
    /// `toolCallId`, which is what nests it.
    #[serde(rename = "parentToolUseId", default)]
    pub parent_tool_use_id: Option<String>,
    /// Present on a dispatch's final update, and nowhere else.
    #[serde(rename = "toolResponse", default)]
    pub tool_response: Option<SubagentResult>,
}

/// What a finished subagent reports about itself.
///
/// Structured on the wire, so read as structure. The same numbers also appear
/// inside the tool's `rawOutput` as a `<usage>` text blob; parsing that would
/// break the first time the adapter reworded it.
#[derive(Debug, Deserialize)]
pub struct SubagentResult {
    #[serde(rename = "agentType", default)]
    pub agent_type: String,
    #[serde(rename = "resolvedModel", default)]
    pub resolved_model: String,
    #[serde(rename = "totalTokens", default)]
    pub total_tokens: u64,
    #[serde(rename = "totalToolUseCount", default)]
    pub total_tool_use_count: u64,
    #[serde(rename = "totalDurationMs", default)]
    pub total_duration_ms: u64,
    #[serde(default)]
    pub status: String,
}
```

Then add this field to each of the `AgentMessageChunk`, `UserMessageChunk`, `AgentThoughtChunk`, `ToolCall`, and `ToolCallUpdate` variants of `SessionUpdate`:

```rust
        #[serde(rename = "_meta", default)]
        meta: Meta,
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent`
Expected: the four new tests PASS. `normalize.rs` will now FAIL to compile — its match arms destructure these variants without the new field. That is Task 2's job; to keep this task independently green, add `meta: _` to the five arms in `normalize.rs` as a mechanical placeholder now.

- [ ] **Step 5: Commit**

```bash
git add crates/agent/src/acp/wire.rs crates/agent/src/acp/normalize.rs
git commit -m "feat(agent): read the subagent structure the adapter sends"
```

---

### Task 2: Events carry the parent pointer

**Files:**
- Modify: `crates/agent/src/event.rs`
- Modify: `crates/agent/src/acp/normalize.rs`
- Modify: `crates/agent/src/session.rs:38,545,570`, `crates/agent/src/link.rs:68`, `crates/agent/src/ring.rs:94`, `crates/agent/src/activity_source.rs:71,75`, `crates/cli/src/agent_host.rs:418`, `crates/daemon/src/agent_supervisor.rs:509,573,608,645,695`

**Interfaces:**
- Consumes: `wire::Meta`, `wire::SubagentResult` from Task 1.
- Produces: `event::SubagentSummary { agent_type: String, model: String, tokens: u64, tool_uses: u64, duration_ms: u64, status: String }`. `AgentEvent::Message` gains `parent: Option<String>`; `AgentEvent::ToolCall` gains `parent: Option<String>` and `subagent: bool`; `AgentEvent::ToolUpdate` gains `parent: Option<String>` and `subagent: Option<SubagentSummary>`.

- [ ] **Step 1: Write the failing tests**

Add to the existing `mod tests` in `crates/agent/src/acp/normalize.rs`:

```rust
    #[test]
    fn a_subagents_message_says_whose_it_is() {
        // The bug this whole change exists for: without the parent, a
        // subagent's words render as the agent that dispatched it talking.
        let raw = r#"{"sessionUpdate":"agent_message_chunk","content":{"text":"I'll read the file."},
            "_meta":{"claudeCode":{"parentToolUseId":"toolu_01Wnr"}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let AgentEvent::Message { parent, .. } = &update_to_events(&update)[0] else {
            panic!("expected a message")
        };
        assert_eq!(parent.as_deref(), Some("toolu_01Wnr"));
    }

    #[test]
    fn a_dispatch_row_is_marked_as_one() {
        let raw = r#"{"sessionUpdate":"tool_call","toolCallId":"t1","title":"Task","kind":"think",
            "status":"pending","_meta":{"claudeCode":{"subagent":true}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let AgentEvent::ToolCall { subagent, .. } = &update_to_events(&update)[0] else {
            panic!("expected a tool call")
        };
        assert!(subagent);
    }

    #[test]
    fn a_finished_dispatch_carries_its_summary() {
        let raw = r#"{"sessionUpdate":"tool_call_update","toolCallId":"t1","status":"completed",
            "_meta":{"claudeCode":{"toolResponse":{"status":"completed","agentType":"general-purpose",
            "resolvedModel":"claude-opus-5[1m]","totalDurationMs":4962,"totalTokens":12479,
            "totalToolUseCount":1}}}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let AgentEvent::ToolUpdate { subagent, .. } = &update_to_events(&update)[0] else {
            panic!("expected a tool update")
        };
        let summary = subagent.as_ref().expect("a summary");
        assert_eq!(summary.agent_type, "general-purpose");
        assert_eq!(summary.tokens, 12479);
        assert_eq!(summary.tool_uses, 1);
        assert_eq!(summary.duration_ms, 4962);
    }

    #[test]
    fn an_ordinary_turn_carries_no_parent_at_all() {
        // Old events in SQLite have no parent field. They must keep decoding,
        // and must keep rendering at the top level.
        let raw = r#"{"sessionUpdate":"agent_message_chunk","content":{"text":"hi"}}"#;
        let update: SessionUpdate = serde_json::from_str(raw).expect("parses");
        let AgentEvent::Message { parent, .. } = &update_to_events(&update)[0] else {
            panic!("expected a message")
        };
        assert!(parent.is_none());
    }

    #[test]
    fn an_event_without_a_parent_serializes_exactly_as_it_used_to() {
        // Stored transcripts and one-release-behind clients both read this
        // JSON. A new key on every ordinary event would bloat every row and
        // change bytes nothing asked to change.
        let event = AgentEvent::Message { role: Role::Agent, text: "hi".into(), parent: None };
        let json = serde_json::to_string(&event).expect("serializes");
        assert_eq!(json, r#"{"Message":{"role":"Agent","text":"hi"}}"#);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent`
Expected: FAIL to compile — no field `parent` / `subagent` on these variants.

- [ ] **Step 3: Add the fields and populate them**

In `crates/agent/src/event.rs`, add above `pub enum AgentEvent`:

```rust
/// What a finished subagent reports about itself, as clients render it.
///
/// A normalized shape rather than the adapter's: the wire's field set is the
/// adapter's to change, and a renderer should not have to track that.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SubagentSummary {
    pub agent_type: String,
    pub model: String,
    pub tokens: u64,
    pub tool_uses: u64,
    pub duration_ms: u64,
    pub status: String,
}
```

Then, in the `AgentEvent` enum, add to `Message`:

```rust
        /// The `ToolCall` id of the dispatch this belongs to, if a subagent
        /// produced it. `None` is the ordinary case: the agent itself spoke.
        ///
        /// Skipped when absent so an ordinary event's JSON is byte-identical
        /// to what it was before subagents were modelled.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
```

to `ToolCall`:

```rust
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
        /// This call IS a subagent dispatch — it owns a block rather than
        /// being a row.
        #[serde(default, skip_serializing_if = "is_false")]
        subagent: bool,
```

and to `ToolUpdate`:

```rust
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
        /// Present once, on the dispatch's final update.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subagent: Option<SubagentSummary>,
```

Add the helper next to `SubagentSummary`:

```rust
/// `skip_serializing_if` needs a predicate by path, and `bool` has no method
/// that fits.
fn is_false(b: &bool) -> bool {
    !*b
}
```

In `crates/agent/src/acp/normalize.rs`, add the conversion and thread `meta` through the five arms:

```rust
fn summary(result: &crate::acp::wire::SubagentResult) -> crate::event::SubagentSummary {
    crate::event::SubagentSummary {
        agent_type: result.agent_type.clone(),
        model: result.resolved_model.clone(),
        tokens: result.total_tokens,
        tool_uses: result.total_tool_use_count,
        duration_ms: result.total_duration_ms,
        status: result.status.clone(),
    }
}
```

Replace the five arms' bodies so each reads `meta`:

```rust
        SessionUpdate::AgentMessageChunk { content, meta } => {
            vec![AgentEvent::Message {
                role: Role::Agent,
                text: content.text.clone(),
                parent: meta.claude_code.parent_tool_use_id.clone(),
            }]
        }
        SessionUpdate::UserMessageChunk { content, meta } => {
            vec![AgentEvent::Message {
                role: Role::User,
                text: content.text.clone(),
                parent: meta.claude_code.parent_tool_use_id.clone(),
            }]
        }
        SessionUpdate::AgentThoughtChunk { content, meta } => {
            vec![AgentEvent::Message {
                role: Role::Thought,
                text: content.text.clone(),
                parent: meta.claude_code.parent_tool_use_id.clone(),
            }]
        }
```

For `ToolCall`, add `meta` to the destructure and set the two new fields on the `AgentEvent::ToolCall`, and on the chained `ToolUpdate` set `parent: meta.claude_code.parent_tool_use_id.clone(), subagent: None`. For `ToolCallUpdate`, add `meta` to the destructure and set:

```rust
                parent: meta.claude_code.parent_tool_use_id.clone(),
                subagent: meta.claude_code.tool_response.as_ref().map(summary),
```

- [ ] **Step 4: Fix every construction site**

Adding a field to a struct variant breaks construction, not pattern matches that use `..`. Add `parent: None` (and `subagent: false` / `subagent: None` where the variant needs it) at each of:

`crates/agent/src/session.rs:38` (`ToolUpdate`), `:545`, `:570` (`Message`); `crates/agent/src/link.rs:68`; `crates/agent/src/ring.rs:94`; `crates/agent/src/activity_source.rs:71,75`; `crates/cli/src/agent_host.rs:418`; `crates/daemon/src/agent_supervisor.rs:509,573,608,645,695`; and the existing assertions in `normalize.rs` at `:199` and `:211`.

- [ ] **Step 5: Run the whole workspace to verify**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test`
Expected: PASS, all crates.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(agent): carry the subagent parent pointer on events"
```

---

### Task 3: Prove it against the real captures

**Files:**
- Create: `crates/agent/tests/subagent_fixtures.rs`
- Delete: `crates/agent/tests/spike_subagent.rs`

**Interfaces:**
- Consumes: `AgentEvent::{Message, ToolCall, ToolUpdate}` with `parent`/`subagent` from Task 2. Fixtures `crates/agent/tests/fixtures/subagent_with_cap.jsonl`, `subagent_without_cap.jsonl` already exist as raw inbound frames, one JSON-RPC frame per line.

- [ ] **Step 1: Write the failing tests**

Create `crates/agent/tests/subagent_fixtures.rs`:

```rust
//! Replay real captured frames from a turn that dispatched a subagent.
//!
//! Captured 2026-08-03 against `@agentclientprotocol/claude-agent-acp` — see
//! `docs/superpowers/specs/2026-08-03-acp-subagent-display-design.md`. Live
//! frames rather than hand-written samples, so a pass here means the
//! normalizer survived contact with the wire.

use farcooler_agent::acp::{normalize::update_to_events, wire};
use farcooler_agent::event::AgentEvent;

fn events_from(fixture: &str) -> Vec<AgentEvent> {
    let raw = std::fs::read_to_string(format!("tests/fixtures/{fixture}")).expect("fixture");
    let mut out = Vec::new();
    for line in raw.lines().filter(|l| !l.trim().is_empty()) {
        let rpc: wire::Rpc = serde_json::from_str(line).expect("frame parses");
        if let Some(n) = rpc.session_notification() {
            out.extend(update_to_events(&n.update));
        }
    }
    out
}

#[test]
fn no_message_of_a_subagents_is_attributed_to_the_agent_that_dispatched_it() {
    // The regression test for the bug this change exists for. Before it, the
    // capture below rendered "I'll read the file." and "3 lines." as the
    // top-level agent speaking, and a reader concluded it had read the file
    // itself.
    let events = events_from("subagent_with_cap.jsonl");
    let nested: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            AgentEvent::Message { text, parent: Some(p), .. } => Some((text.as_str(), p.as_str())),
            _ => None,
        })
        .collect();
    assert!(
        nested.iter().any(|(t, _)| t.contains("I'll read the file")),
        "the subagent's own narration was not attributed to it: {events:#?}"
    );
    let dispatch = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::ToolCall { id, subagent: true, .. } => Some(id.clone()),
            _ => None,
        })
        .expect("the capture dispatched a subagent");
    for (text, parent) in &nested {
        assert_eq!(parent, &dispatch, "{text:?} named a parent that is not the dispatch");
    }
}

#[test]
fn a_finished_dispatch_reports_what_the_subagent_cost() {
    let events = events_from("subagent_with_cap.jsonl");
    let summary = events
        .iter()
        .find_map(|e| match e {
            AgentEvent::ToolUpdate { subagent: Some(s), .. } => Some(s),
            _ => None,
        })
        .expect("a completed dispatch");
    assert_eq!(summary.agent_type, "general-purpose");
    assert!(summary.tokens > 0, "no token count: {summary:?}");
    assert!(summary.duration_ms > 0, "no duration: {summary:?}");
}

#[test]
fn a_subagents_tool_calls_nest_even_without_the_transcript_capability() {
    // Measured, and the reason the capability is not load-bearing: with
    // `subagent-transcript` off the adapter withholds the subagent's
    // NARRATION but still tags its tool calls. Turning the flag off would
    // therefore not have fixed the mis-nesting.
    let events = events_from("subagent_without_cap.jsonl");
    assert!(
        events.iter().any(|e| matches!(e, AgentEvent::ToolCall { parent: Some(_), .. })),
        "no parented tool call in the without-capability capture: {events:#?}"
    );
    assert!(
        !events.iter().any(|e| matches!(e, AgentEvent::Message { parent: Some(_), .. })),
        "the without-capability capture should carry no subagent narration"
    );
}

#[test]
fn a_subagent_turn_produces_no_gaps() {
    // Nesting must not be bought with false "history missing" breaks. This is
    // the property that made the original bug invisible, and it has to stay
    // true for the right reason now.
    for fixture in ["subagent_with_cap.jsonl", "subagent_without_cap.jsonl"] {
        let events = events_from(fixture);
        assert!(
            !events.iter().any(|e| matches!(e, AgentEvent::Gap { .. })),
            "{fixture} produced a gap"
        );
    }
}

#[test]
fn a_workflow_is_one_opaque_row_and_says_so() {
    // Pins the finding that scoped workflows out: a Workflow tool call is
    // not a dispatch and emits nothing for its internal agents. If a future
    // adapter starts tagging them, this test fails and the workflow spec
    // becomes reachable.
    let events = events_from("workflow_dispatch.jsonl");
    assert!(
        !events.iter().any(|e| matches!(e, AgentEvent::ToolCall { subagent: true, .. })),
        "a workflow now marks itself as a subagent dispatch — revisit the spec"
    );
    assert!(
        !events.iter().any(|e| matches!(
            e,
            AgentEvent::Message { parent: Some(_), .. } | AgentEvent::ToolCall { parent: Some(_), .. }
        )),
        "a workflow now emits parented frames — revisit the spec"
    );
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent --test subagent_fixtures`
Expected: FAIL — before Task 2 lands they would not compile; after Task 2 they should pass. If any fail here, the normalizer is wrong, not the test.

- [ ] **Step 3: Delete the spike**

```bash
git rm crates/agent/tests/spike_subagent.rs
```

- [ ] **Step 4: Run the suite**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "test(agent): pin subagent nesting against live captures"
```

---

### Task 4: Swift decodes the new fields

**Files:**
- Modify: `apps/shared/AgentKit/Sources/AgentKit/AgentEvent.swift`
- Modify: `apps/shared/AgentKit/Tests/AgentKitTests/AgentEventTests.swift`

**Interfaces:**
- Consumes: the JSON shape Task 2 produces.
- Produces: `public struct SubagentSummary: Decodable, Sendable, Equatable` with `agentType, model, tokens, toolUses, durationMs, status`. `AgentEvent.message` gains `parent: String?`; `.toolCall` gains `parent: String?, subagent: Bool`; `.toolUpdate` gains `parent: String?, subagent: SubagentSummary?`. All three are the LAST associated values, in that order.

- [ ] **Step 1: Write the failing tests**

Add to `apps/shared/AgentKit/Tests/AgentKitTests/AgentEventTests.swift`:

```swift
@Test func aSubagentsMessageDecodesWithItsParent() throws {
    // Without the parent the client cannot tell a subagent's words from the
    // dispatching agent's, which is exactly how they came to be rendered as
    // the same speaker.
    let json = #"{"Message":{"role":"Agent","text":"I'll read the file.","parent":"toolu_01Wnr"}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .message(_, text, parent) = event else {
        Issue.record("expected a message, got \(event)")
        return
    }
    #expect(text == "I'll read the file.")
    #expect(parent == "toolu_01Wnr")
}

@Test func aMessageWithoutAParentStillDecodes() throws {
    // Every event already in SQLite was written before this field existed.
    // If absence threw, every stored transcript would fail to render.
    let json = #"{"Message":{"role":"Agent","text":"hello"}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .message(_, _, parent) = event else {
        Issue.record("expected a message, got \(event)")
        return
    }
    #expect(parent == nil)
}

@Test func aDispatchDecodesAsOne() throws {
    let json = #"{"ToolCall":{"id":"t1","title":"Task","kind":"think","status":"Pending","locations":[],"subagent":true}}"#
    let event = try AgentEvent.decode(from: json)
    guard case let .toolCall(_, _, _, _, _, _, subagent) = event else {
        Issue.record("expected a tool call, got \(event)")
        return
    }
    #expect(subagent)
}

@Test func aFinishedDispatchDecodesItsSummary() throws {
    // These are the numbers the collapsed row shows. Dropping them leaves a
    // finished subagent saying only that it finished.
    let json = """
        {"ToolUpdate":{"id":"t1","status":"Completed","title":null,"content":null,\
        "diff":null,"locations":[],"subagent":{"agent_type":"general-purpose",\
        "model":"claude-opus-5[1m]","tokens":12479,"tool_uses":1,"duration_ms":4962,\
        "status":"completed"}}}
        """
    let event = try AgentEvent.decode(from: json)
    guard case let .toolUpdate(_, _, _, _, _, _, _, summary) = event else {
        Issue.record("expected a tool update, got \(event)")
        return
    }
    #expect(summary?.agentType == "general-purpose")
    #expect(summary?.tokens == 12479)
    #expect(summary?.toolUses == 1)
    #expect(summary?.durationMs == 4962)
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test --package-path apps/shared/AgentKit`
Expected: FAIL to compile — the cases take fewer associated values.

- [ ] **Step 3: Add the type and extend the cases**

In `AgentEvent.swift`, add beside the other supporting types:

```swift
/// What a finished subagent reports about itself.
public struct SubagentSummary: Decodable, Sendable, Equatable {
    public let agentType: String
    public let model: String
    public let tokens: UInt64
    public let toolUses: UInt64
    public let durationMs: UInt64
    public let status: String

    public init(
        agentType: String, model: String, tokens: UInt64, toolUses: UInt64,
        durationMs: UInt64, status: String
    ) {
        self.agentType = agentType
        self.model = model
        self.tokens = tokens
        self.toolUses = toolUses
        self.durationMs = durationMs
        self.status = status
    }

    enum CodingKeys: String, CodingKey {
        case agentType = "agent_type"
        case model, tokens, status
        case toolUses = "tool_uses"
        case durationMs = "duration_ms"
    }
}
```

Change the three enum cases to:

```swift
    case message(role: Role, text: String, parent: String?)
    case toolCall(
        id: String, title: String, kind: String, status: ToolStatus, locations: [String],
        parent: String?, subagent: Bool)
    case toolUpdate(
        id: String, status: ToolStatus, title: String?, content: String?, diff: Diff?,
        locations: [String], parent: String?, subagent: SubagentSummary?)
```

Extend the payload structs — every new property optional or defaulted so an old event still decodes:

```swift
    private struct MessagePayload: Decodable {
        let role: Role
        let text: String
        let parent: String?
    }
    private struct ToolCallPayload: Decodable {
        let id: String; let title: String; let kind: String
        let status: ToolStatus; let locations: [String]
        let parent: String?
        let subagent: Bool?
    }
    private struct ToolUpdatePayload: Decodable {
        let id: String
        let status: ToolStatus
        let title: String?
        let content: String?
        let diff: Diff?
        let locations: [String]
        let parent: String?
        let subagent: SubagentSummary?
    }
```

And update the three `Envelope` switch arms:

```swift
            case "Message":
                let p = try outer.decode(MessagePayload.self, forKey: key)
                event = .message(role: p.role, text: p.text, parent: p.parent)
            case "ToolCall":
                let p = try outer.decode(ToolCallPayload.self, forKey: key)
                event = .toolCall(
                    id: p.id, title: p.title, kind: p.kind, status: p.status,
                    locations: p.locations, parent: p.parent, subagent: p.subagent ?? false)
            case "ToolUpdate":
                let p = try outer.decode(ToolUpdatePayload.self, forKey: key)
                event = .toolUpdate(
                    id: p.id, status: p.status, title: p.title, content: p.content,
                    diff: p.diff, locations: p.locations, parent: p.parent,
                    subagent: p.subagent)
```

- [ ] **Step 4: Fix every existing construction and pattern site**

`Transcript.swift:191,203,208` bind positionally and will not compile. Every construction in `TranscriptTests.swift` (lines 11, 12, 27, 28, 37, 38, 52, 53, 68, 70, 103, 150, 152, 161, 162, 212, 214, 251, 253) and the pattern at `AgentEventTests.swift:7` need the new values. For this task, add `parent: nil`, `subagent: false`, `subagent: nil` mechanically; Task 5 rewrites the `Transcript.swift` arms properly.

- [ ] **Step 5: Run to verify they pass**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test --package-path apps/shared/AgentKit`
Expected: PASS, all tests.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(agentkit): decode the subagent parent and summary"
```

---

### Task 5: The transcript nests

**Files:**
- Modify: `apps/shared/AgentKit/Sources/AgentKit/Transcript.swift`
- Modify: `apps/shared/AgentKit/Tests/AgentKitTests/TranscriptTests.swift`

**Interfaces:**
- Consumes: `AgentEvent` cases from Task 4.
- Produces: `public struct SubagentBlock: Sendable, Equatable, Identifiable` with `tool: ToolRow`, `children: [TranscriptRow]`, `summary: SubagentSummary?`, `interrupted: Bool`, and `id: String { tool.id }`. `TranscriptRow.Kind` gains `case subagent(SubagentBlock)`.

- [ ] **Step 1: Write the failing tests**

Add to `apps/shared/AgentKit/Tests/AgentKitTests/TranscriptTests.swift`:

```swift
@Test func aSubagentsWorkLivesInsideTheCallThatDispatchedIt() {
    // The bug in one test: before this, the subagent's message and its Read
    // rendered as siblings of the Task row, and the transcript claimed the
    // top-level agent had read the file itself.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "task1", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(1, .message(role: .agent, text: "I'll read the file.", parent: "task1")),
        seq(2, .toolCall(id: "r1", title: "Read main.rs", kind: "read", status: .completed,
                         locations: [], parent: "task1", subagent: false)),
    ])
    #expect(t.rows.count == 1)
    guard case let .subagent(block) = t.rows[0].kind else {
        Issue.record("expected a subagent block, got \(t.rows[0].kind)")
        return
    }
    #expect(block.children.count == 2)
    guard case let .message(_, text, _) = block.children[0].kind else {
        Issue.record("expected the subagent's message inside the block")
        return
    }
    #expect(text == "I'll read the file.")
}

@Test func aToolInsideABlockIsUpdatedInPlaceRatherThanDuplicated() {
    // The flat lookup found nothing for a nested tool and appended a second,
    // half-built row beside the block — one tool rendering as two.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "task1", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(1, .toolCall(id: "r1", title: "Read", kind: "read", status: .pending,
                         locations: [], parent: "task1", subagent: false)),
        seq(2, .toolUpdate(id: "r1", status: .completed, title: "Read main.rs", content: "3 lines",
                           diff: nil, locations: [], parent: "task1", subagent: nil)),
    ])
    #expect(t.rows.count == 1)
    guard case let .subagent(block) = t.rows[0].kind else {
        Issue.record("expected a subagent block")
        return
    }
    #expect(block.children.count == 1)
    guard case let .tool(tool) = block.children[0].kind else {
        Issue.record("expected a tool row inside the block")
        return
    }
    #expect(tool.status == .completed)
    #expect(tool.title == "Read main.rs")
}

@Test func twoSubagentsRunningAtOnceDoNotBleedIntoEachOther() {
    // Frames from parallel subagents interleave in one stream. Routing by
    // "most recent block" instead of by parent id would file each line under
    // whichever subagent spoke last.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "a", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(1, .toolCall(id: "b", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(2, .message(role: .agent, text: "from a", parent: "a")),
        seq(3, .message(role: .agent, text: "from b", parent: "b")),
        seq(4, .message(role: .agent, text: " still a", parent: "a")),
    ])
    #expect(t.rows.count == 2)
    guard case let .subagent(first) = t.rows[0].kind,
        case let .subagent(second) = t.rows[1].kind
    else {
        Issue.record("expected two blocks")
        return
    }
    guard case let .message(_, textA, _) = first.children[0].kind,
        case let .message(_, textB, _) = second.children[0].kind
    else {
        Issue.record("expected a message in each block")
        return
    }
    #expect(textA == "from a still a")
    #expect(textB == "from b")
}

@Test func aSubagentsWordsNeverJoinTheDispatchingAgentsSentence() {
    // Message chunks coalesce with the previous row. Coalescing across a
    // parent boundary would splice a subagent's sentence onto the end of the
    // agent's own, attributing it to the wrong speaker in the worst way.
    var t = Transcript()
    t.apply([
        seq(0, .message(role: .agent, text: "I'll dispatch.", parent: nil)),
        seq(1, .toolCall(id: "task1", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(2, .message(role: .agent, text: "inside", parent: "task1")),
    ])
    #expect(t.rows.count == 2)
    guard case let .message(_, text, _) = t.rows[0].kind else {
        Issue.record("expected the agent's own message on top")
        return
    }
    #expect(text == "I'll dispatch.")
}

@Test func aFrameWhoseParentWasNeverSeenIsShownRatherThanLost() {
    // A trimmed ring or a partial reload can leave a child with no block to
    // hang from. Rendering it at the top level is honest — nothing is
    // missing but the nesting. Dropping it would shorten the transcript
    // silently, and a gap would claim content was lost when none was.
    var t = Transcript()
    t.apply([seq(0, .message(role: .agent, text: "orphaned", parent: "never-seen"))])
    #expect(t.rows.count == 1)
    guard case let .message(_, text, _) = t.rows[0].kind else {
        Issue.record("expected the orphan to render as a top-level message")
        return
    }
    #expect(text == "orphaned")
}

@Test func aSubagentCutOffMidRunDoesNotRenderAsOneThatSucceeded() {
    // A cancelled turn never delivers the dispatch's completion. Leaving the
    // block as-is would show a subagent whose fate nobody knows wearing the
    // same mark as one that reported back.
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "task1", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(1, .turnEnded(reason: "Cancelled")),
    ])
    guard case let .subagent(block) = t.rows[0].kind else {
        Issue.record("expected a subagent block")
        return
    }
    #expect(block.interrupted)
}

@Test func aFinishedBlockCarriesItsSummary() {
    var t = Transcript()
    t.apply([
        seq(0, .toolCall(id: "task1", title: "Task", kind: "think", status: .inProgress,
                         locations: [], parent: nil, subagent: true)),
        seq(1, .toolUpdate(id: "task1", status: .completed, title: "Count lines", content: nil,
                           diff: nil, locations: [], parent: nil,
                           subagent: SubagentSummary(
                               agentType: "general-purpose", model: "claude-opus-5[1m]",
                               tokens: 12479, toolUses: 1, durationMs: 4962, status: "completed"))),
    ])
    guard case let .subagent(block) = t.rows[0].kind else {
        Issue.record("expected a subagent block")
        return
    }
    #expect(block.summary?.tokens == 12479)
    #expect(block.tool.status == .completed)
    #expect(!block.interrupted)
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test --package-path apps/shared/AgentKit`
Expected: FAIL — no `.subagent` case on `Kind`.

- [ ] **Step 3: Add the block type**

In `Transcript.swift`, after `ToolRow`:

```swift
/// A subagent's dispatch row and everything it did.
///
/// The children are `TranscriptRow`s rather than a second row type, so a
/// subagent's messages, tools, and gaps render through exactly the same views
/// as the top level. Nesting is where they live, not what they are.
public struct SubagentBlock: Sendable, Equatable, Identifiable {
    public var id: String { tool.id }
    /// The `Task` call itself: title, status, location.
    public var tool: ToolRow
    public var children: [TranscriptRow]
    /// What it reported on finishing. Absent while it runs.
    public var summary: SubagentSummary?
    /// The turn ended before this reported back, so its outcome is unknown.
    /// Distinct from failure, and emphatically distinct from success.
    public var interrupted: Bool
}
```

Add to `Kind`: `case subagent(SubagentBlock)`.

- [ ] **Step 4: Add routing**

In `Transcript.swift`, replace `append(_ kind:)` and add the routing helpers:

```swift
    /// Where a row belongs.
    ///
    /// An unknown parent resolves to `.top` rather than being dropped: the
    /// content is all present, only its nesting is unknown, and a shorter
    /// transcript that looks complete is the one failure this design refuses.
    private enum Destination {
        case top
        case block(Int)
    }

    private func destination(for parent: String?) -> Destination {
        guard let parent else { return .top }
        guard
            let index = rows.lastIndex(where: {
                if case let .subagent(block) = $0.kind { return block.tool.id == parent }
                return false
            })
        else { return .top }
        return .block(index)
    }

    private mutating func append(_ kind: TranscriptRow.Kind, to destination: Destination = .top) {
        let row = TranscriptRow(id: nextRowID, kind: kind)
        nextRowID += 1
        switch destination {
        case .top:
            rows.append(row)
        case let .block(index):
            guard case var .subagent(block) = rows[index].kind else { return }
            block.children.append(row)
            rows[index].kind = .subagent(block)
        }
    }

    /// The last row of whichever container this destination names.
    private func lastRow(in destination: Destination) -> TranscriptRow? {
        switch destination {
        case .top:
            return rows.last
        case let .block(index):
            guard case let .subagent(block) = rows[index].kind else { return nil }
            return block.children.last
        }
    }

    private mutating func replaceLastRow(in destination: Destination, with kind: TranscriptRow.Kind) {
        switch destination {
        case .top:
            rows[rows.count - 1].kind = kind
        case let .block(index):
            guard case var .subagent(block) = rows[index].kind, !block.children.isEmpty else { return }
            block.children[block.children.count - 1].kind = kind
            rows[index].kind = .subagent(block)
        }
    }

    /// The merge rules for a tool update, in one place so the three lookup
    /// paths cannot drift apart.
    private func merged(
        _ tool: ToolRow, status: ToolStatus, title: String?, content: String?, diff: Diff?,
        locations: [String]
    ) -> ToolRow {
        var tool = tool
        tool.status = status
        if let title, !title.isEmpty { tool.title = title }
        if let content { tool.content = content }
        if let diff { tool.diff = diff }
        if !locations.isEmpty { tool.locations = locations }
        return tool
    }
```

- [ ] **Step 5: Rewrite the three event arms**

Replace the `.message`, `.toolCall`, and `.toolUpdate` arms of `private mutating func apply(_ event:)`:

```swift
        case let .message(role, text, parent):
            let target = destination(for: parent)
            // Chunks of one message coalesce, but only within one container.
            // Across a parent boundary this would splice a subagent's
            // sentence onto the dispatching agent's.
            if case let .message(lastRole, lastText, lastParent) = lastRow(in: target)?.kind,
                lastRole == role, lastParent == parent,
                !(parent == nil && breakBeforeNextMessage)
            {
                replaceLastRow(in: target, with: .message(role: role, text: lastText + text, parent: parent))
            } else {
                append(.message(role: role, text: text, parent: parent), to: target)
            }
            if parent == nil { breakBeforeNextMessage = false }

        case let .toolCall(id, title, kind, status, locations, parent, subagent):
            let tool = ToolRow(
                id: id, title: title, kind: kind, status: status,
                locations: locations, content: nil, diff: nil)
            if subagent {
                // A dispatch owns a block rather than being a row in one.
                append(.subagent(SubagentBlock(
                    tool: tool, children: [], summary: nil, interrupted: false)))
            } else {
                append(.tool(tool), to: destination(for: parent))
            }

        case let .toolUpdate(id, status, newTitle, content, diff, locations, parent, summary):
            applyToolUpdate(
                id: id, status: status, title: newTitle, content: content, diff: diff,
                locations: locations, parent: parent, summary: summary)
```

Add the lookup, which must try three places before giving up:

```swift
    /// Update a call in place, wherever it lives.
    ///
    /// Three lookups, in order: the dispatch rows themselves, the top level,
    /// then inside a block. The flat search this replaced found nothing for a
    /// nested tool and appended a second half-built row beside the block, so
    /// one tool rendered as two.
    private mutating func applyToolUpdate(
        id: String, status: ToolStatus, title: String?, content: String?, diff: Diff?,
        locations: [String], parent: String?, summary: SubagentSummary?
    ) {
        if let index = rows.lastIndex(where: {
            if case let .subagent(block) = $0.kind { return block.tool.id == id }
            return false
        }) {
            guard case var .subagent(block) = rows[index].kind else { return }
            block.tool = merged(
                block.tool, status: status, title: title, content: content, diff: diff,
                locations: locations)
            if let summary { block.summary = summary }
            rows[index].kind = .subagent(block)
            return
        }

        if let index = rows.lastIndex(where: {
            if case let .tool(tool) = $0.kind { return tool.id == id }
            return false
        }) {
            guard case let .tool(tool) = rows[index].kind else { return }
            rows[index].kind = .tool(merged(
                tool, status: status, title: title, content: content, diff: diff,
                locations: locations))
            return
        }

        for index in rows.indices.reversed() {
            guard case var .subagent(block) = rows[index].kind else { continue }
            guard
                let child = block.children.lastIndex(where: {
                    if case let .tool(tool) = $0.kind { return tool.id == id }
                    return false
                })
            else { continue }
            guard case let .tool(tool) = block.children[child].kind else { continue }
            block.children[child].kind = .tool(merged(
                tool, status: status, title: title, content: content, diff: diff,
                locations: locations))
            rows[index].kind = .subagent(block)
            return
        }

        append(
            .tool(ToolRow(
                id: id, title: title ?? id, kind: "", status: status, locations: locations,
                content: content, diff: diff)),
            to: destination(for: parent))
    }
```

- [ ] **Step 6: Resolve running blocks when the turn ends**

Replace the `.turnEnded` arm:

```swift
        case .turnEnded:
            breakBeforeNextMessage = true
            // A subagent still running when the turn ends never gets its
            // completion. Left alone it would keep a spinner forever or, once
            // the view stopped animating, read as one that finished.
            for index in rows.indices {
                guard case var .subagent(block) = rows[index].kind else { continue }
                guard block.tool.status == .pending || block.tool.status == .inProgress else { continue }
                block.interrupted = true
                rows[index].kind = .subagent(block)
            }
```

- [ ] **Step 7: Run to verify they pass**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift test --package-path apps/shared/AgentKit`
Expected: PASS. The app targets will not compile yet — their `switch row.kind` is exhaustive. Tasks 6 and 7 fix that.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(agentkit): nest a subagent's work inside its dispatch"
```

---

### Task 6: macOS renders the block

**Files:**
- Modify: `apps/macos/Sources/FarCooler/AgentRows.swift:22-31`
- Modify: `apps/macos/Sources/FarCooler/AgentSurface.swift:263-282`

**Interfaces:**
- Consumes: `SubagentBlock`, `TranscriptRow.Kind.subagent` from Task 5.
- Produces: `SubagentBlockView` (internal, in `AgentRows.swift`).

- [ ] **Step 1: Add the case to the row switch**

In `AgentRows.swift`, add to `AgentRowView.body`:

```swift
        case let .subagent(block):
            SubagentBlockView(block: block)
```

- [ ] **Step 2: Write the block view**

Add to `AgentRows.swift`. This reuses the file's existing disclosure idiom (`ThoughtRow` at `:173`, `ToolRowView` at `:226`) rather than a `DisclosureGroup`, which was rejected for tinting labels with the accent color:

```swift
/// A subagent's dispatch and everything it did.
///
/// Open while it works, closed once it reports — the same rule `ThoughtRow`
/// uses, and for the same reason: the interesting moment is while it happens,
/// and a finished one is noise until you ask. The difference is that a reader
/// who touches it wins permanently, because a block that shut itself while
/// someone was reading it is worse than one that stayed open.
private struct SubagentBlockView: View {
    let block: SubagentBlock

    /// `nil` means nobody has said, so the automatic rule applies.
    @State private var toggled: Bool?
    @State private var showingAll = false

    /// How many children a running block shows. Enough to see what it is
    /// doing; few enough that six at once still fit on a screen.
    private static let visibleChildren = 3

    private var running: Bool { block.tool.status == .pending || block.tool.status == .inProgress }
    private var showing: Bool { toggled ?? running }

    private var shown: [TranscriptRow] {
        guard !showingAll, block.children.count > Self.visibleChildren else { return block.children }
        return Array(block.children.suffix(Self.visibleChildren))
    }

    private var hidden: Int { block.children.count - shown.count }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(Motion.snap) { toggled = !showing }
            } label: {
                header.contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if showing && !block.children.isEmpty {
                Divider()
                VStack(alignment: .leading, spacing: 10) {
                    if hidden > 0 {
                        Button("… \(hidden) more") { withAnimation(Motion.snap) { showingAll = true } }
                            .buttonStyle(.plain)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    ForEach(shown) { child in
                        AgentRowView(row: child)
                    }
                }
                .padding(9)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .background(.quinary, in: RoundedRectangle(cornerRadius: 7))
        .animation(Motion.snap, value: showing)
        .animation(Motion.snap, value: block.children.count)
    }

    private var header: some View {
        HStack(spacing: 7) {
            Image(systemName: "chevron.right")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .rotationEffect(.degrees(showing ? 90 : 0))
            StatusGlyph(status: status, size: 7)
            Text(block.tool.title)
                .font(.callout.weight(.medium))
                .lineLimit(1)
                .truncationMode(.middle)
            Text(subtitle)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 4)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// What a collapsed block still answers without being opened.
    private var subtitle: String {
        if block.interrupted { return "interrupted" }
        guard let summary = block.summary else {
            return running ? "\(block.children.count) steps" : ""
        }
        let tokens = summary.tokens >= 1000
            ? "\(summary.tokens / 1000)k tok"
            : "\(summary.tokens) tok"
        let seconds = String(format: "%.1fs", Double(summary.durationMs) / 1000)
        return "\(summary.agentType) · \(summary.toolUses) tools · \(tokens) · \(seconds)"
    }

    private var status: Status {
        if block.interrupted { return .failed }
        switch block.tool.status {
        case .pending: return .starting
        case .inProgress: return .working
        case .completed: return .done
        case .failed: return .failed
        }
    }
}
```

- [ ] **Step 3: Make permission lookup see into blocks**

`AgentSurface.swift:263-282` matches only top-level `.tool` rows, so a permission raised by a tool inside a block would render unattached. In `permission(gating:)` add a `.subagent` arm that checks the block's children, and in `unattachedPermission` extend the `rows.contains` predicate to search children too:

```swift
        case let .subagent(block):
            return block.children.contains { child in
                if case let .tool(tool) = child.kind { return tool.id == pending.toolCall }
                return false
            } ? pending : nil
```

- [ ] **Step 4: Build**

Run: `DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer swift build --package-path apps/shared/AgentKit && ./apps/macos/build-app.sh`
Expected: builds clean.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(macos): render a subagent as a block of its own"
```

---

### Task 7: iOS renders the block

**Files:**
- Modify: `apps/ios/FarCooler/AgentView.swift:207-216,294-318,40-59`

**Interfaces:**
- Consumes: `SubagentBlock`, `TranscriptRow.Kind.subagent` from Task 5.
- Produces: `SubagentBlockView` (private, in `AgentView.swift`).

- [ ] **Step 1: Add the case to the row switch**

At `AgentView.swift:309-316`, add:

```swift
        case let .subagent(block):
            SubagentBlockView(block: block)
```

- [ ] **Step 2: Write the block view**

Add to `AgentView.swift`, matching this file's constants — `Color.primary.opacity(0.07)`, corner radius 8, and its own spring, since `Motion` is a Mac-target type:

```swift
/// A subagent's dispatch and everything it did. The Mac's block, on a phone.
private struct SubagentBlockView: View {
    let block: SubagentBlock

    /// `nil` means nobody has said, so the automatic rule applies: open while
    /// it works, closed once it reports. A reader who touches it wins
    /// permanently — a block that shut itself mid-read is worse than one left
    /// open.
    @State private var toggled: Bool?
    @State private var showingAll = false

    private static let visibleChildren = 3
    private static let motion = Animation.spring(response: 0.22, dampingFraction: 0.82)

    private var running: Bool { block.tool.status == .pending || block.tool.status == .inProgress }
    private var showing: Bool { toggled ?? running }

    private var shown: [TranscriptRow] {
        guard !showingAll, block.children.count > Self.visibleChildren else { return block.children }
        return Array(block.children.suffix(Self.visibleChildren))
    }

    private var hidden: Int { block.children.count - shown.count }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Button {
                withAnimation(Self.motion) { toggled = !showing }
            } label: {
                header.contentShape(Rectangle())
            }
            .buttonStyle(.plain)

            if showing && !block.children.isEmpty {
                Divider()
                VStack(alignment: .leading, spacing: 10) {
                    if hidden > 0 {
                        Button("… \(hidden) more") { withAnimation(Self.motion) { showingAll = true } }
                            .buttonStyle(.plain)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                    }
                    ForEach(shown) { child in
                        AgentRowView(row: child)
                    }
                }
                .padding(9)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .background(Color.primary.opacity(0.07), in: RoundedRectangle(cornerRadius: 8))
        .animation(Self.motion, value: showing)
        .animation(Self.motion, value: block.children.count)
    }

    private var header: some View {
        HStack(spacing: 7) {
            Image(systemName: "chevron.right")
                .font(.caption2)
                .foregroundStyle(.secondary)
                .rotationEffect(.degrees(showing ? 90 : 0))
            Circle()
                .fill(dotColor)
                .frame(width: 7, height: 7)
            Text(block.tool.title)
                .font(.callout.weight(.medium))
                .lineLimit(1)
                .truncationMode(.middle)
            Text(subtitle)
                .font(.caption)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Spacer(minLength: 4)
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private var subtitle: String {
        if block.interrupted { return "interrupted" }
        guard let summary = block.summary else {
            return running ? "\(block.children.count) steps" : ""
        }
        let tokens = summary.tokens >= 1000
            ? "\(summary.tokens / 1000)k tok"
            : "\(summary.tokens) tok"
        let seconds = String(format: "%.1fs", Double(summary.durationMs) / 1000)
        return "\(summary.agentType) · \(summary.toolUses) tools · \(tokens) · \(seconds)"
    }

    private var dotColor: Color {
        if block.interrupted { return .red }
        switch block.tool.status {
        case .pending: return .secondary.opacity(0.35)
        case .inProgress: return .secondary
        case .completed: return .green
        case .failed: return .red
        }
    }
}
```

- [ ] **Step 3: Give iOS rows a stable identity**

`AgentView.swift:207-216` has no `.id(row.id)` on the row, unlike macOS at `AgentSurface.swift:205`. In a `LazyVStack` that recycles views, the `@State` holding a reader's manual toggle can be handed to a different block. Add `.id(row.id)` to the `AgentRowView` in the `ForEach`.

- [ ] **Step 4: Make permission lookup see into blocks**

Apply the same `.subagent` arm as Task 6 Step 3 to `permission(gating:)` at `AgentView.swift:40-45` and `unattachedPermission` at `:52-59`.

- [ ] **Step 5: Build**

Run:
```bash
python3 apps/ios/generate-project.py
DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer xcodebuild \
  -project apps/ios/FarCooler.xcodeproj -scheme FarCooler \
  -destination 'generic/platform=iOS Simulator' build
```
Expected: BUILD SUCCEEDED.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(ios): render a subagent as a block of its own"
```

---

## Self-Review

**Spec coverage.** Wire `_meta` → Task 1. Event fields, no proto change → Task 2. Orphan-renders-top-level → Task 5 Step 4 (`destination` returns `.top`) and tested in Task 5 Step 1. Epoch reset → already correct: `resetForNewEpoch()` clears `rows`, and children hang off `rows`, so no separate map exists to go stale. Cancelled turns → Task 5 Step 6. Auto-expand with manual override → Tasks 6 and 7 (`toggled: Bool?`). Line cap of 3 → Tasks 6 and 7 (`visibleChildren`). Capability stays on → no change to `session.rs:225`, deliberately. Fixture tests → Task 3. Spike deleted → Task 3 Step 3.

**Deviation from the spec, deliberate.** The spec called for a `toolCallId → row index` map. `resetForNewEpoch` would have had to clear it, and it could disagree with `rows`. Since blocks live in `rows` and there are rarely more than a handful, the three-way lookup in Task 5 Step 5 searches `rows` directly — no second structure, nothing to keep in step. The spec's stated goal (no ordering assumptions, no state machine) holds.

**Open question carried forward.** Whether a subagent's `session/request_permission` carries parent attribution is still unverified. Tasks 6 and 7 Step 3 make the lookup search children, which is correct whether or not the request is attributed — it matches on the tool call id, which is present either way.
