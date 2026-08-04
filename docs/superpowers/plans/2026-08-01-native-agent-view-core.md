# Native Agent View — Core Contract Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and test the complete host-side contract for running a coding agent headlessly under the Agent Client Protocol inside a tmux pane, so that a later plan can render it natively on macOS and iOS.

**Architecture:** A terminal gains a `pane_mode`. In `AGENT` pane mode its tagged tmux pane runs `farcooler agent-host`, a shim that spawns an ACP adapter, answers the ACP `fs` capability under worktree confinement, owns a bounded event ring, and bridges normalized events to the daemon over a Unix socket. Liveness derivation is untouched — the pane is still the sole authority. No transcript is persisted.

**Tech Stack:** Rust 2024 (workspace crates), tokio, prost/protobuf for client-facing protocol, serde_json for the internal shim socket, rusqlite for durable intent, tmux control for pane lifecycle.

**Source spec:** `docs/superpowers/specs/2026-08-01-native-agent-view-design.md`. This plan covers spec slices 1–5. Slices 6–8 (Mac and iOS UI) are a separate plan written against the contract this one settles.

## Global Constraints

- Rust edition `2024`, `rust-version = "1.85"`, workspace resolver `3`. New crates inherit via `[workspace.package]`.
- Three distinct names, never the bare word alone: `mode` = VT modes (existing, untouched); `pane_mode` = TERMINAL vs AGENT; `agent_mode` = the ACP concept.
- No conversation content is ever written to SQLite. SQLite stores `pane_mode` and `agent_session_id` only, as durable intent.
- Missing history is always an explicit `Gap` event. Never a shorter transcript, never a silent drop.
- `derive.rs` is not modified. Liveness stays derived from a live exactly-tagged pane.
- Client-facing protocol is protobuf in `proto/farcooler.proto`. The shim↔daemon socket is newline-delimited JSON, permitted because shim and daemon ship in the same binary and therefore have no version skew.
- Every `fs/read_text_file` and `fs/write_text_file` path is fully resolved, symlinks included, and rejected unless inside the worktree the shim was launched for.
- Error messages crossing the client protocol never contain a path, terminal byte, command, or session id (existing rule, `proto/farcooler.proto:83`).

---

### Task 1: Gate 1 — prove the ACP adapter maps to a resumable Claude session

**This task can kill the design. Do it first and stop if it fails.** Everything downstream assumes the ACP adapter's `sessionId` corresponds to a Claude Code session that `claude --resume` will open. If it does not, the pane-mode toggle silently shows the wrong conversation.

**Files:**
- Create: `docs/superpowers/specs/2026-08-01-gate1-acp-findings.md`
- Create: `crates/agent/tests/fixtures/session_basic.jsonl`
- Create: `crates/agent/tests/fixtures/session_permission.jsonl`

- [ ] **Step 1: Install the adapter and capture a session**

```bash
mkdir -p /tmp/gate1 && cd /tmp/gate1 && git init -q && echo "fn main() {}" > main.rs && git add -A && git commit -qm init
npx -y @zed-industries/claude-code-acp --version
```

- [ ] **Step 2: Drive one turn over ACP by hand, recording both directions**

Write `/tmp/gate1/drive.mjs`. It speaks newline-delimited JSON-RPC 2.0 on the adapter's stdio, declares the `fs` capability, and logs every frame to `capture.jsonl`.

```javascript
import { spawn } from "node:child_process";
import { appendFileSync } from "node:fs";

const p = spawn("npx", ["-y", "@zed-industries/claude-code-acp"], { stdio: ["pipe", "pipe", "inherit"] });
const log = (dir, msg) => appendFileSync("capture.jsonl", JSON.stringify({ dir, msg }) + "\n");
const send = (msg) => { log("out", msg); p.stdin.write(JSON.stringify(msg) + "\n"); };

let buf = "";
p.stdout.on("data", (d) => {
  buf += d;
  let i;
  while ((i = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, i); buf = buf.slice(i + 1);
    if (!line.trim()) continue;
    const msg = JSON.parse(line);
    log("in", msg);
    // Answer client-side requests so the turn can proceed.
    if (msg.method === "fs/read_text_file") send({ jsonrpc: "2.0", id: msg.id, result: { content: "fn main() {}\n" } });
    if (msg.method === "fs/write_text_file") send({ jsonrpc: "2.0", id: msg.id, result: {} });
    if (msg.method === "session/request_permission")
      send({ jsonrpc: "2.0", id: msg.id, result: { outcome: { outcome: "selected", optionId: msg.params.options[0].optionId } } });
  }
});

send({ jsonrpc: "2.0", id: 1, method: "initialize",
  params: { protocolVersion: 1, clientCapabilities: { fs: { readTextFile: true, writeTextFile: true } } } });
setTimeout(() => send({ jsonrpc: "2.0", id: 2, method: "session/new", params: { cwd: "/tmp/gate1", mcpServers: [] } }), 1500);
setTimeout(() => send({ jsonrpc: "2.0", id: 3, method: "session/prompt",
  params: { sessionId: process.env.SID, prompt: [{ type: "text", text: "Add a doc comment to main.rs" }] } }), 4000);
```

Run it, reading the `sessionId` out of the `session/new` result and re-running with `SID=<that> node drive.mjs` for the prompt leg:

```bash
cd /tmp/gate1 && node drive.mjs
```

- [ ] **Step 3: Check the four gate conditions against `capture.jsonl`**

```bash
cd /tmp/gate1
grep -c '"fs/write_text_file"' capture.jsonl        # (1) must be >= 1
grep -c '"session/request_permission"' capture.jsonl # (2) must be >= 1
ls ~/.claude/projects/-tmp-gate1/                    # (4) must list <sessionId>.jsonl
```

For (3), start a second run with `session/load` against the recorded `sessionId` and confirm it replays `session/update` notifications rather than erroring.

For (4), the decisive check — the ACP `sessionId` must name a real Claude session:

```bash
claude --resume <sessionId> -p "say only: RESUMED" --output-format text
```

- [ ] **Step 4: Record the findings**

Write `docs/superpowers/specs/2026-08-01-gate1-acp-findings.md` stating, for each of the four conditions, PASS or FAIL with the evidence line from `capture.jsonl`. If condition (4) fails, record what the adapter's `sessionId` *is* and whether a Claude session id can be recovered by watching `~/.claude/projects/<munged-cwd>/` for a file created during `session/new` — that mapping becomes an extra field on the shim and the rest of the plan proceeds unchanged.

**Stop and report to the user if (1) or (2) fails.** Those are not recoverable by a fallback: without `fs/write_text_file` there are no protocol diffs, and without `session/request_permission` there are no structured approvals, which together are why ACP was chosen over Claude's native stream-json.

- [ ] **Step 5: Convert the capture into test fixtures**

Extract only the inbound frames into two fixture files, one per scenario, one JSON object per line:

```bash
cd /tmp/gate1
mkdir -p ~/Dev/farcooler/crates/agent/tests/fixtures
jq -c 'select(.dir=="in") | .msg' capture.jsonl > ~/Dev/farcooler/crates/agent/tests/fixtures/session_basic.jsonl
```

Then produce `session_permission.jsonl` the same way from a run whose prompt triggers an approval (ask it to run a shell command). These fixtures are what every later test replays, so CI never needs credentials or a live agent.

- [ ] **Step 6: Commit**

```bash
git add docs/superpowers/specs/2026-08-01-gate1-acp-findings.md crates/agent/tests/fixtures/
git commit -m "spike: gate 1 — ACP adapter session identity and fs capability"
```

---

### Task 2: The `farcooler-agent` crate and its normalized event model

**Files:**
- Modify: `Cargo.toml` (workspace members and dependencies)
- Create: `crates/agent/Cargo.toml`
- Create: `crates/agent/src/lib.rs`
- Create: `crates/agent/src/event.rs`

**Interfaces:**
- Produces: `farcooler_agent::event::{AgentEvent, Sequenced, Role, ToolStatus, Diff, PlanEntry, PermissionOption, EndReason, AgentGapReason, Seq}`.

- [ ] **Step 1: Write the failing test**

Create `crates/agent/src/event.rs` containing only this test module at the bottom of the file (the types come in step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gap_is_a_first_class_event_not_an_absence() {
        // The whole reason a derived transcript is allowed to exist in this
        // product: it can say where it is incomplete. A missing range must be
        // representable, or callers will express it as a shorter list and the
        // UI will render a lie.
        let e = AgentEvent::Gap { reason: AgentGapReason::RingTrimmed };
        assert!(matches!(e, AgentEvent::Gap { .. }));
    }

    #[test]
    fn a_sequenced_event_carries_its_own_position() {
        // Clients subscribe from a cursor, so an event that does not know its
        // own seq cannot be replayed into the right place.
        let s = Sequenced { seq: 7, event: AgentEvent::TurnEnded { reason: EndReason::EndTurn } };
        assert_eq!(s.seq, 7);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-agent`
Expected: FAIL — `error: package ID specification 'farcooler-agent' did not match any packages`

- [ ] **Step 3: Create the crate and the model**

Add to `Cargo.toml` `[workspace] members` after `"crates/core",`:

```toml
    "crates/agent",
```

Add to `[workspace.dependencies]`:

```toml
farcooler-agent = { path = "crates/agent" }
serde = { version = "1", features = ["derive"] }
```

Create `crates/agent/Cargo.toml`:

```toml
[package]
name = "farcooler-agent"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
farcooler-protocol.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
tracing.workspace = true
```

Create `crates/agent/src/lib.rs`:

```rust
//! The agent view's host side: one normalized event model, and the adapters
//! that produce it.
//!
//! Clients never see a vendor protocol. That is the entire point — the UI is
//! written once against `event::AgentEvent`, and a new agent is a new adapter
//! rather than a new screen.

pub mod event;
```

Prepend to `crates/agent/src/event.rs`, above the test module written in step 1:

```rust
//! What an agent did, in terms no vendor owns.

/// Position in a session's event stream. Monotonic, starts at 0.
pub type Seq = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    User,
    Agent,
    /// Reasoning the agent showed its working for. Collapsed by default in UI.
    Thought,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// An edit, as before-and-after rather than as a reconstruction.
///
/// This exists only because the client answers `fs/write_text_file`. Rebuilding
/// it from tool-call arguments would couple this crate to each agent's private
/// tool schemas, which is the coupling ACP was chosen to avoid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diff {
    pub path: String,
    /// `None` when the file did not exist before the write.
    pub old_text: Option<String>,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanEntry {
    pub content: String,
    pub priority: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionOption {
    pub id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EndReason {
    EndTurn,
    Cancelled,
    Refusal,
    MaxTokens,
}

/// Why history is missing. Named so a client can explain itself to a user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentGapReason {
    /// The ring dropped events the subscriber had not read.
    RingTrimmed,
    /// Reconnected, but the agent does not implement `session/load`.
    LoadUnsupported,
    /// An update arrived that this adapter could not interpret.
    Unparsed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    SessionStarted {
        session_id: String,
        agent_mode: Option<String>,
        available_modes: Vec<String>,
        available_commands: Vec<String>,
    },
    Message {
        role: Role,
        text: String,
    },
    ToolCall {
        id: String,
        title: String,
        kind: String,
        status: ToolStatus,
        locations: Vec<String>,
    },
    ToolUpdate {
        id: String,
        status: ToolStatus,
        content: Option<String>,
        diff: Option<Diff>,
    },
    Plan {
        entries: Vec<PlanEntry>,
    },
    Permission {
        id: String,
        tool_call: String,
        options: Vec<PermissionOption>,
    },
    Resolved {
        id: String,
        chosen: String,
    },
    ModeSet {
        agent_mode: String,
    },
    TurnEnded {
        reason: EndReason,
    },
    Gap {
        reason: AgentGapReason,
    },
}

/// An event and where it sits in the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sequenced {
    pub seq: Seq,
    pub event: AgentEvent,
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-agent`
Expected: PASS, 2 tests

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/agent/
git commit -m "feat(agent): one normalized event model, so the UI never sees a vendor"
```

---

### Task 3: Normalize ACP `session/update` into `AgentEvent`

**Files:**
- Create: `crates/agent/src/acp/mod.rs`
- Create: `crates/agent/src/acp/wire.rs`
- Create: `crates/agent/src/acp/normalize.rs`
- Create: `crates/agent/tests/normalize_fixtures.rs`
- Modify: `crates/agent/src/lib.rs`

**Interfaces:**
- Consumes: `event::{AgentEvent, Role, ToolStatus, PlanEntry, PermissionOption, AgentGapReason}` (Task 2).
- Produces: `farcooler_agent::acp::normalize::update_to_events(&wire::SessionUpdate) -> Vec<AgentEvent>` and `farcooler_agent::acp::wire::{Rpc, SessionUpdate, SessionNotification}`.

- [ ] **Step 1: Write the failing test**

Create `crates/agent/tests/normalize_fixtures.rs`:

```rust
//! Replay real captured ACP frames. No live agent, no credentials in CI.

use farcooler_agent::acp::{normalize::update_to_events, wire};
use farcooler_agent::event::{AgentEvent, Role};

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
fn a_captured_session_yields_agent_messages() {
    let events = events_from("session_basic.jsonl");
    assert!(
        events.iter().any(|e| matches!(e, AgentEvent::Message { role: Role::Agent, .. })),
        "no agent message in a real captured turn: {events:#?}"
    );
}

#[test]
fn a_captured_session_yields_tool_calls() {
    let events = events_from("session_basic.jsonl");
    assert!(
        events.iter().any(|e| matches!(e, AgentEvent::ToolCall { .. })),
        "no tool call in a turn that edited a file"
    );
}

#[test]
fn an_update_nobody_wrote_a_rule_for_becomes_a_gap_not_a_silence() {
    // The honesty rule, at the one place it is cheapest to break: an unknown
    // variant must be visible, or a future adapter change silently shortens
    // every transcript.
    let json = r#"{"sessionUpdate":"something_from_a_future_version"}"#;
    let update: wire::SessionUpdate = serde_json::from_str(json).expect("parses as unknown");
    let events = update_to_events(&update);
    assert_eq!(events, vec![AgentEvent::Gap { reason: farcooler_agent::event::AgentGapReason::Unparsed }]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-agent --test normalize_fixtures`
Expected: FAIL — `unresolved import farcooler_agent::acp`

- [ ] **Step 3: Write the wire types and the normalizer**

Add `pub mod acp;` to `crates/agent/src/lib.rs`.

Create `crates/agent/src/acp/mod.rs`:

```rust
//! The Agent Client Protocol adapter.

pub mod normalize;
pub mod wire;
```

Create `crates/agent/src/acp/wire.rs`:

```rust
//! ACP frames, deserialized only as far as the normalizer needs.
//!
//! Deliberately lenient: unknown fields are ignored and unknown update kinds
//! deserialize to `Unknown` rather than failing the frame. A strict decoder
//! would turn every adapter release into an outage.

use serde::Deserialize;

/// One JSON-RPC frame from the adapter.
#[derive(Debug, Deserialize)]
pub struct Rpc {
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<serde_json::Value>,
    #[serde(default)]
    pub id: Option<serde_json::Value>,
    #[serde(default)]
    pub result: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SessionNotification {
    #[serde(rename = "sessionId", default)]
    pub session_id: String,
    pub update: SessionUpdate,
}

impl Rpc {
    /// The frame as a `session/update` notification, if that is what it is.
    pub fn session_notification(&self) -> Option<SessionNotification> {
        if self.method.as_deref() != Some("session/update") {
            return None;
        }
        serde_json::from_value(self.params.clone()?).ok()
    }
}

#[derive(Debug, Deserialize)]
pub struct ContentBlock {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Deserialize)]
pub struct WirePlanEntry {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub priority: String,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub enum SessionUpdate {
    AgentMessageChunk { content: ContentBlock },
    UserMessageChunk { content: ContentBlock },
    AgentThoughtChunk { content: ContentBlock },
    ToolCall {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(default)]
        title: String,
        #[serde(default)]
        kind: String,
        #[serde(default)]
        status: String,
        #[serde(default)]
        locations: Vec<Location>,
    },
    ToolCallUpdate {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(default)]
        status: String,
    },
    Plan {
        #[serde(default)]
        entries: Vec<WirePlanEntry>,
    },
    CurrentModeUpdate {
        #[serde(rename = "currentModeId")]
        current_mode_id: String,
    },
    /// Anything this version does not know. Becomes a visible `Gap`.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
pub struct Location {
    #[serde(default)]
    pub path: String,
}
```

Create `crates/agent/src/acp/normalize.rs`:

```rust
//! ACP updates in, normalized events out.

use crate::acp::wire::{SessionUpdate, WirePlanEntry};
use crate::event::{AgentEvent, AgentGapReason, PlanEntry, Role, ToolStatus};

fn status(raw: &str) -> ToolStatus {
    match raw {
        "in_progress" => ToolStatus::InProgress,
        "completed" => ToolStatus::Completed,
        "failed" => ToolStatus::Failed,
        _ => ToolStatus::Pending,
    }
}

fn plan_entry(e: &WirePlanEntry) -> PlanEntry {
    PlanEntry { content: e.content.clone(), priority: e.priority.clone(), status: e.status.clone() }
}

/// One update becomes zero or more events.
///
/// Never an empty vec for an update that carried meaning: an update this
/// version cannot interpret produces a `Gap`, because a silently dropped
/// update is a transcript that is wrong without saying so.
pub fn update_to_events(update: &SessionUpdate) -> Vec<AgentEvent> {
    match update {
        SessionUpdate::AgentMessageChunk { content } => {
            vec![AgentEvent::Message { role: Role::Agent, text: content.text.clone() }]
        }
        SessionUpdate::UserMessageChunk { content } => {
            vec![AgentEvent::Message { role: Role::User, text: content.text.clone() }]
        }
        SessionUpdate::AgentThoughtChunk { content } => {
            vec![AgentEvent::Message { role: Role::Thought, text: content.text.clone() }]
        }
        SessionUpdate::ToolCall { tool_call_id, title, kind, status: s, locations } => {
            vec![AgentEvent::ToolCall {
                id: tool_call_id.clone(),
                title: title.clone(),
                kind: kind.clone(),
                status: status(s),
                locations: locations.iter().map(|l| l.path.clone()).collect(),
            }]
        }
        SessionUpdate::ToolCallUpdate { tool_call_id, status: s } => {
            vec![AgentEvent::ToolUpdate {
                id: tool_call_id.clone(),
                status: status(s),
                content: None,
                diff: None,
            }]
        }
        SessionUpdate::Plan { entries } => {
            vec![AgentEvent::Plan { entries: entries.iter().map(plan_entry).collect() }]
        }
        SessionUpdate::CurrentModeUpdate { current_mode_id } => {
            vec![AgentEvent::ModeSet { agent_mode: current_mode_id.clone() }]
        }
        SessionUpdate::Unknown => vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }],
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-agent --test normalize_fixtures`
Expected: PASS, 3 tests

If the two fixture-driven tests fail because the captured field names differ from those above, correct `wire.rs` to match the fixture — the fixture is real and this file is the guess.

- [ ] **Step 5: Commit**

```bash
git add crates/agent/src/acp crates/agent/src/lib.rs crates/agent/tests/normalize_fixtures.rs
git commit -m "feat(agent): normalize ACP updates, and say Gap when we cannot"
```

---

### Task 4: The bounded event ring with cursor replay

**Files:**
- Create: `crates/agent/src/ring.rs`
- Modify: `crates/agent/src/lib.rs`

**Interfaces:**
- Consumes: `event::{AgentEvent, Sequenced, Seq, AgentGapReason}` (Task 2).
- Produces: `farcooler_agent::ring::{AgentRing, AgentReplay, AGENT_RING_EVENTS}` with `AgentRing::new()`, `push(&mut self, AgentEvent) -> Seq`, `next_seq(&self) -> Seq`, `since(&self, from: Seq) -> AgentReplay`.

- [ ] **Step 1: Write the failing test**

Create `crates/agent/src/ring.rs` containing only this test module (implementation in step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AgentEvent, AgentGapReason, EndReason, Role};

    fn msg(text: &str) -> AgentEvent {
        AgentEvent::Message { role: Role::Agent, text: text.to_string() }
    }

    #[test]
    fn sequence_numbers_are_monotonic_from_zero() {
        let mut ring = AgentRing::new();
        assert_eq!(ring.push(msg("a")), 0);
        assert_eq!(ring.push(msg("b")), 1);
        assert_eq!(ring.next_seq(), 2);
    }

    #[test]
    fn replay_from_a_cursor_returns_exactly_what_came_after_it() {
        let mut ring = AgentRing::new();
        ring.push(msg("a"));
        ring.push(msg("b"));
        ring.push(msg("c"));
        let AgentReplay::At { events } = ring.since(1) else { panic!("expected retained replay") };
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[1].seq, 2);
    }

    #[test]
    fn replay_from_the_head_is_empty_not_a_gap() {
        // A caller that is fully caught up must not be told it lost history.
        let mut ring = AgentRing::new();
        ring.push(msg("a"));
        let AgentReplay::At { events } = ring.since(1) else { panic!("expected retained replay") };
        assert!(events.is_empty());
    }

    #[test]
    fn a_trimmed_ring_reports_a_gap_rather_than_a_shorter_transcript() {
        // The rule this whole crate exists to keep. Dropping the oldest events
        // silently would give a client a transcript that is wrong and looks
        // complete.
        let mut ring = AgentRing::with_capacity(2);
        ring.push(msg("a"));
        ring.push(msg("b"));
        ring.push(msg("c"));
        let AgentReplay::Gap { resumed_at, dropped, events } = ring.since(0) else {
            panic!("expected a gap")
        };
        assert_eq!(resumed_at, 1);
        assert_eq!(dropped, 1);
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn a_gap_event_can_itself_be_stored_and_replayed() {
        // `session/load` unsupported on reconnect pushes a Gap into the stream;
        // it must survive replay like any other event.
        let mut ring = AgentRing::new();
        ring.push(AgentEvent::Gap { reason: AgentGapReason::LoadUnsupported });
        ring.push(AgentEvent::TurnEnded { reason: EndReason::EndTurn });
        let AgentReplay::At { events } = ring.since(0) else { panic!("expected retained replay") };
        assert!(matches!(events[0].event, AgentEvent::Gap { .. }));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-agent ring`
Expected: FAIL — `cannot find type AgentRing in this scope`

- [ ] **Step 3: Write the ring**

Add `pub mod ring;` to `crates/agent/src/lib.rs`. Prepend to `crates/agent/src/ring.rs`:

```rust
//! A session's recent events, bounded, replayable from a cursor.
//!
//! This is the terminal `ReplayBuffer`'s idea at a different granularity:
//! terminals count bytes because a byte offset is the only honest cursor into a
//! stream with no record boundaries, and agent events count events because they
//! have them. What is shared is the part that matters — dropping history is
//! always visible to the reader.
//!
//! It lives in the SHIM, not the daemon. The shim is co-located with the agent
//! and dies with the pane whose liveness is already authoritative, so a daemon
//! restart never costs a transcript.

use std::collections::VecDeque;

use crate::event::{AgentEvent, Seq, Sequenced};

/// Events retained per session. At roughly a few hundred bytes each this is a
/// small number of megabytes for an farcooler session, and trimming is visible.
pub const AGENT_RING_EVENTS: usize = 4096;

#[derive(Debug, PartialEq, Eq)]
pub enum AgentReplay {
    /// The cursor was still retained; these are the events after it.
    At { events: Vec<Sequenced> },
    /// The cursor had been trimmed. The reader must render a break before
    /// applying `events`.
    Gap { resumed_at: Seq, dropped: u64, events: Vec<Sequenced> },
}

#[derive(Debug)]
pub struct AgentRing {
    events: VecDeque<Sequenced>,
    capacity: usize,
    next_seq: Seq,
}

impl AgentRing {
    pub fn new() -> Self {
        Self::with_capacity(AGENT_RING_EVENTS)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self { events: VecDeque::new(), capacity: capacity.max(1), next_seq: 0 }
    }

    /// The seq the next pushed event will take.
    pub fn next_seq(&self) -> Seq {
        self.next_seq
    }

    /// The oldest seq still retained.
    pub fn oldest_seq(&self) -> Seq {
        self.events.front().map(|e| e.seq).unwrap_or(self.next_seq)
    }

    pub fn push(&mut self, event: AgentEvent) -> Seq {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.events.push_back(Sequenced { seq, event });
        while self.events.len() > self.capacity {
            self.events.pop_front();
        }
        seq
    }

    /// Everything from `from` onwards, or a gap if `from` is no longer held.
    pub fn since(&self, from: Seq) -> AgentReplay {
        let oldest = self.oldest_seq();
        if from < oldest {
            return AgentReplay::Gap {
                resumed_at: oldest,
                dropped: oldest - from,
                events: self.events.iter().cloned().collect(),
            };
        }
        AgentReplay::At {
            events: self.events.iter().filter(|e| e.seq >= from).cloned().collect(),
        }
    }
}

impl Default for AgentRing {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-agent ring`
Expected: PASS, 5 tests

- [ ] **Step 5: Commit**

```bash
git add crates/agent/src/ring.rs crates/agent/src/lib.rs
git commit -m "feat(agent): a bounded ring that admits what it dropped"
```

---

### Task 5: Path confinement for the `fs` capability

**Files:**
- Create: `crates/agent/src/fs_guard.rs`
- Modify: `crates/agent/src/lib.rs`

**Interfaces:**
- Produces: `farcooler_agent::fs_guard::{confine, FsGuardError}` with `confine(worktree: &Path, requested: &Path) -> Result<PathBuf, FsGuardError>`.

This is the security boundary named in the spec. Taking the ACP `fs` capability means Far Cooler writes files an agent names, on a host reachable from a phone. Without confinement it is an arbitrary-write primitive.

- [ ] **Step 1: Write the failing test**

Create `crates/agent/src/fs_guard.rs` containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn worktree() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("farcooler-guard-{}", std::process::id()));
        let _ = fs::create_dir_all(dir.join("src"));
        fs::canonicalize(&dir).expect("temp worktree")
    }

    #[test]
    fn a_path_inside_the_worktree_is_allowed() {
        let wt = worktree();
        let ok = confine(&wt, &wt.join("src/main.rs")).expect("inside is allowed");
        assert!(ok.starts_with(&wt));
    }

    #[test]
    fn a_file_that_does_not_exist_yet_is_allowed_inside() {
        // Every create goes through this. Requiring the file to exist would
        // make the capability useless for new files.
        let wt = worktree();
        assert!(confine(&wt, &wt.join("src/brand_new.rs")).is_ok());
    }

    #[test]
    fn dot_dot_cannot_climb_out() {
        let wt = worktree();
        let err = confine(&wt, &wt.join("../../etc/passwd")).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }

    #[test]
    fn an_absolute_path_elsewhere_is_refused() {
        let wt = worktree();
        let err = confine(&wt, std::path::Path::new("/etc/passwd")).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }

    #[test]
    fn a_symlink_pointing_out_is_refused() {
        // The one that a naive prefix check on the unresolved string misses,
        // and the reason resolution has to happen before comparison.
        let wt = worktree();
        let link = wt.join("escape");
        let _ = fs::remove_file(&link);
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc", &link).expect("symlink");
        let err = confine(&wt, &link.join("passwd")).unwrap_err();
        assert!(matches!(err, FsGuardError::Escapes));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-agent fs_guard`
Expected: FAIL — `cannot find function confine in this scope`

- [ ] **Step 3: Write the guard**

Add `pub mod fs_guard;` to `crates/agent/src/lib.rs`. Prepend to `crates/agent/src/fs_guard.rs`:

```rust
//! Where an agent is allowed to read and write.
//!
//! Resolution before comparison, always. A prefix check on the path as written
//! passes `worktree/escape/passwd` when `escape` is a symlink to `/etc`, which
//! is the whole attack. The deepest existing ancestor is canonicalized and the
//! remaining components are appended, so a file that does not exist yet is
//! still judged by where it would actually land.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FsGuardError {
    #[error("path resolves outside the workspace worktree")]
    Escapes,
    #[error("worktree path could not be resolved")]
    BadWorktree,
}

/// Resolve `requested` and return it only if it lands inside `worktree`.
pub fn confine(worktree: &Path, requested: &Path) -> Result<PathBuf, FsGuardError> {
    let root = std::fs::canonicalize(worktree).map_err(|_| FsGuardError::BadWorktree)?;

    let absolute =
        if requested.is_absolute() { requested.to_path_buf() } else { root.join(requested) };

    // Canonicalize the deepest ancestor that exists, then re-append the rest.
    // `canonicalize` on a missing path fails outright, and every file creation
    // is a missing path.
    let mut existing = absolute.as_path();
    let mut tail: Vec<Component<'_>> = Vec::new();
    let resolved_head = loop {
        match std::fs::canonicalize(existing) {
            Ok(p) => break p,
            Err(_) => match existing.parent() {
                Some(parent) => {
                    if let Some(name) = existing.components().next_back() {
                        tail.push(name);
                    }
                    existing = parent;
                }
                None => return Err(FsGuardError::Escapes),
            },
        }
    };

    let mut resolved = resolved_head;
    for component in tail.into_iter().rev() {
        match component {
            // A `..` that survived to here would climb out of the resolved
            // head, so it is refused rather than normalized away.
            Component::ParentDir => return Err(FsGuardError::Escapes),
            Component::CurDir => {}
            other => resolved.push(other.as_os_str()),
        }
    }

    if resolved.starts_with(&root) { Ok(resolved) } else { Err(FsGuardError::Escapes) }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-agent fs_guard`
Expected: PASS, 5 tests

- [ ] **Step 5: Commit**

```bash
git add crates/agent/src/fs_guard.rs crates/agent/src/lib.rs
git commit -m "feat(agent): an agent writes only inside its own worktree"
```

---

### Task 6: The ACP connection — JSON-RPC over the adapter's stdio

**Files:**
- Create: `crates/agent/src/acp/conn.rs`
- Modify: `crates/agent/src/acp/mod.rs`

**Interfaces:**
- Consumes: `acp::wire::Rpc` (Task 3), `fs_guard::confine` (Task 5).
- Produces: `farcooler_agent::acp::conn::{AcpConnection, Incoming, AcpError}` with `AcpConnection::spawn(program, args, worktree) -> Result<Self>`, `request(&mut self, method, params) -> Result<serde_json::Value>`, `notify(&mut self, method, params) -> Result<()>`, `next_incoming(&mut self) -> Option<Incoming>`, `respond(&mut self, id, result) -> Result<()>`.

- [ ] **Step 1: Write the failing test**

Create `crates/agent/src/acp/conn.rs` containing only this test module. It drives a fake adapter — `cat` echoes nothing useful, so the test uses a tiny shell script that replies to one request — proving framing without needing the real adapter.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A fake adapter: reads one line, writes one JSON-RPC result.
    fn fake_adapter() -> (String, Vec<String>) {
        (
            "/bin/sh".to_string(),
            vec![
                "-c".to_string(),
                r#"read line; printf '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}\n'"#
                    .to_string(),
            ],
        )
    }

    #[tokio::test]
    async fn a_request_is_matched_to_its_response_by_id() {
        let (program, args) = fake_adapter();
        let mut conn = AcpConnection::spawn(&program, &args, std::env::temp_dir())
            .await
            .expect("spawn fake adapter");
        let result = conn
            .request("initialize", serde_json::json!({ "protocolVersion": 1 }))
            .await
            .expect("a result comes back");
        assert_eq!(result["protocolVersion"], 1);
    }

    #[tokio::test]
    async fn frames_are_newline_delimited_json() {
        // ACP is line-delimited JSON-RPC on stdio. A framing mistake here shows
        // up as a hang rather than an error, so it is asserted directly.
        let line = encode_frame(&serde_json::json!({ "jsonrpc": "2.0", "method": "x" }));
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-agent acp::conn`
Expected: FAIL — `cannot find type AcpConnection in this scope`

- [ ] **Step 3: Write the connection**

Add `pub mod conn;` to `crates/agent/src/acp/mod.rs`. Prepend to `crates/agent/src/acp/conn.rs`:

```rust
//! One ACP adapter process, and the JSON-RPC conversation with it.
//!
//! Line-delimited JSON, both directions. The connection is bidirectional in a
//! way LSP clients often are not: the adapter sends US requests — `fs/*` and
//! `session/request_permission` — and a client that only pumps responses
//! deadlocks the agent mid-turn. Hence `next_incoming`.

use std::path::PathBuf;
use std::process::Stdio;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use crate::acp::wire::Rpc;

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("could not start the ACP adapter")]
    Spawn,
    #[error("the ACP adapter closed its connection")]
    Closed,
    #[error("malformed frame from the ACP adapter")]
    Malformed,
}

/// A frame the ADAPTER sent us that is not a response to our request.
///
/// `id` is `Some` for a request we must answer (`fs/*`,
/// `session/request_permission`) and `None` for a notification
/// (`session/update`). Both must reach the caller: a notification-only queue
/// deadlocks the agent, and a request-only queue drops the entire transcript.
#[derive(Debug)]
pub struct Incoming {
    pub id: Option<serde_json::Value>,
    pub method: String,
    pub params: serde_json::Value,
}

/// One frame, as it goes on the wire.
pub fn encode_frame(value: &serde_json::Value) -> String {
    format!("{value}\n")
}

pub struct AcpConnection {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
    pending_incoming: Vec<Incoming>,
    pub worktree: PathBuf,
}

impl AcpConnection {
    pub async fn spawn(
        program: &str,
        args: &[String],
        worktree: impl Into<PathBuf>,
    ) -> Result<Self, AcpError> {
        let worktree = worktree.into();
        let mut child = Command::new(program)
            .args(args)
            .current_dir(&worktree)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|_| AcpError::Spawn)?;
        let stdin = child.stdin.take().ok_or(AcpError::Spawn)?;
        let stdout = BufReader::new(child.stdout.take().ok_or(AcpError::Spawn)?);
        Ok(Self { child, stdin, stdout, next_id: 1, pending_incoming: Vec::new(), worktree })
    }

    async fn write(&mut self, value: serde_json::Value) -> Result<(), AcpError> {
        self.stdin
            .write_all(encode_frame(&value).as_bytes())
            .await
            .map_err(|_| AcpError::Closed)?;
        self.stdin.flush().await.map_err(|_| AcpError::Closed)
    }

    pub async fn notify(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), AcpError> {
        self.write(serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params }))
            .await
    }

    pub async fn respond(
        &mut self,
        id: serde_json::Value,
        result: serde_json::Value,
    ) -> Result<(), AcpError> {
        self.write(serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result })).await
    }

    /// Send a request and read until its response arrives.
    ///
    /// Frames that are not the answer are not discarded: notifications and
    /// adapter-initiated requests are queued for `next_incoming`, because
    /// dropping a `session/request_permission` here would hang the agent.
    pub async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, AcpError> {
        let id = self.next_id;
        self.next_id += 1;
        self.write(serde_json::json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))
            .await?;

        loop {
            let frame = self.read_frame().await?;
            let rpc: Rpc = serde_json::from_str(&frame).map_err(|_| AcpError::Malformed)?;
            if rpc.id.as_ref().and_then(|v| v.as_u64()) == Some(id) && rpc.result.is_some() {
                return Ok(rpc.result.unwrap_or(serde_json::Value::Null));
            }
            self.queue(rpc);
        }
    }

    /// The next adapter-initiated request, if one has arrived.
    pub async fn next_incoming(&mut self) -> Result<Option<Incoming>, AcpError> {
        if let Some(i) = self.pending_incoming.pop() {
            return Ok(Some(i));
        }
        let frame = self.read_frame().await?;
        let rpc: Rpc = serde_json::from_str(&frame).map_err(|_| AcpError::Malformed)?;
        self.queue(rpc);
        Ok(self.pending_incoming.pop())
    }

    /// Queue anything the adapter initiated.
    ///
    /// Keyed on `method`, not on `id`: a `session/update` notification carries
    /// no id, and an earlier version of this filtered on id and therefore threw
    /// away every message the agent produced while keeping only its questions.
    fn queue(&mut self, rpc: Rpc) {
        let Some(method) = rpc.method.clone() else { return };
        if rpc.result.is_some() {
            return;
        }
        self.pending_incoming.push(Incoming {
            id: rpc.id.clone(),
            method,
            params: rpc.params.unwrap_or(serde_json::Value::Null),
        });
    }

    async fn read_frame(&mut self) -> Result<String, AcpError> {
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line).await.map_err(|_| AcpError::Closed)?;
        if n == 0 { Err(AcpError::Closed) } else { Ok(line) }
    }

    pub async fn kill(&mut self) {
        let _ = self.child.kill().await;
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-agent acp::conn`
Expected: PASS, 2 tests

- [ ] **Step 5: Commit**

```bash
git add crates/agent/src/acp/conn.rs crates/agent/src/acp/mod.rs
git commit -m "feat(agent): a connection that answers the adapter, not just itself"
```

---

### Task 7: The session driver — turn a connection into a stream of events

**Files:**
- Create: `crates/agent/src/session.rs`
- Modify: `crates/agent/src/lib.rs`

**Interfaces:**
- Consumes: `acp::conn::{AcpConnection, Incoming}` (Task 6), `acp::normalize::update_to_events` (Task 3), `fs_guard::confine` (Task 5), `ring::AgentRing` (Task 4).
- Produces: `farcooler_agent::session::{AgentSession, SessionError, handle_fs_read, handle_fs_write, permission_event, load_unsupported_event}` with `AgentSession::start(conn: AcpConnection, resume: Option<String>) -> Result<(Self, Vec<AgentEvent>), SessionError>`, `pump(&mut self) -> Result<Vec<AgentEvent>, SessionError>`, `prompt(&mut self, text: &str)`, `answer(&mut self, request_id: serde_json::Value, option_id: &str)`, `set_mode(&mut self, agent_mode: &str)`, `cancel(&mut self)`. Public fields `session_id: String`, `available_modes: Vec<String>`, `available_commands: Vec<String>`.

- [ ] **Step 1: Write the failing test**

Create `crates/agent/src/session.rs` containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AgentEvent, AgentGapReason};

    #[test]
    fn an_fs_write_becomes_a_diff_carrying_what_was_there_before() {
        // Tier 2's whole justification: the diff is a protocol fact, not a
        // reconstruction from a vendor's private tool schema.
        let dir = std::env::temp_dir().join(format!("farcooler-sess-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.txt");
        std::fs::write(&file, "old\n").unwrap();

        let event = handle_fs_write(&dir, file.to_str().unwrap(), "new\n").expect("allowed");
        let AgentEvent::ToolUpdate { diff: Some(d), .. } = event else { panic!("expected a diff") };
        assert_eq!(d.old_text.as_deref(), Some("old\n"));
        assert_eq!(d.new_text, "new\n");
    }

    #[test]
    fn a_write_outside_the_worktree_is_refused_and_says_so() {
        let dir = std::env::temp_dir().join(format!("farcooler-sess2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(handle_fs_write(&dir, "/etc/passwd", "x").is_err());
    }

    #[test]
    fn a_permission_request_becomes_a_blocking_event() {
        let params = serde_json::json!({
            "toolCall": { "toolCallId": "t1", "title": "Run ls" },
            "options": [
                { "optionId": "allow", "name": "Yes", "kind": "allow_once" },
                { "optionId": "reject", "name": "No", "kind": "reject_once" }
            ]
        });
        let event = permission_event("req-1", &params);
        let AgentEvent::Permission { id, options, .. } = event else { panic!("expected permission") };
        assert_eq!(id, "req-1");
        assert_eq!(options.len(), 2);
        assert_eq!(options[0].id, "allow");
    }

    #[test]
    fn a_reconnect_without_session_load_produces_a_visible_gap() {
        assert_eq!(
            load_unsupported_event(),
            AgentEvent::Gap { reason: AgentGapReason::LoadUnsupported }
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-agent session`
Expected: FAIL — `cannot find function handle_fs_write in this scope`

- [ ] **Step 3: Write the driver**

Add `pub mod session;` to `crates/agent/src/lib.rs`. Prepend to `crates/agent/src/session.rs`:

```rust
//! One ACP session: the conversation, the capability answers, and the events.

use std::path::Path;

use crate::acp::conn::{AcpConnection, AcpError, Incoming};
use crate::acp::normalize::update_to_events;
use crate::acp::wire::Rpc;
use crate::event::{AgentEvent, AgentGapReason, Diff, PermissionOption, ToolStatus};
use crate::fs_guard::confine;

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error(transparent)]
    Acp(#[from] AcpError),
    #[error("refused: the path is outside the workspace worktree")]
    Refused,
    #[error("the agent did not accept the session")]
    Rejected,
}

/// Perform a confined write and describe it as a diff.
pub fn handle_fs_write(
    worktree: &Path,
    requested: &str,
    contents: &str,
) -> Result<AgentEvent, SessionError> {
    let path = confine(worktree, Path::new(requested)).map_err(|_| SessionError::Refused)?;
    let old_text = std::fs::read_to_string(&path).ok();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(&path, contents).map_err(|_| SessionError::Refused)?;
    Ok(AgentEvent::ToolUpdate {
        id: path.display().to_string(),
        status: ToolStatus::Completed,
        content: None,
        diff: Some(Diff {
            path: path.display().to_string(),
            old_text,
            new_text: contents.to_string(),
        }),
    })
}

/// Read a confined file for the agent.
pub fn handle_fs_read(worktree: &Path, requested: &str) -> Result<String, SessionError> {
    let path = confine(worktree, Path::new(requested)).map_err(|_| SessionError::Refused)?;
    std::fs::read_to_string(&path).map_err(|_| SessionError::Refused)
}

/// A `session/request_permission` as the event that blocks a fleet row.
pub fn permission_event(request_id: &str, params: &serde_json::Value) -> AgentEvent {
    let options = params["options"]
        .as_array()
        .map(|opts| {
            opts.iter()
                .map(|o| PermissionOption {
                    id: o["optionId"].as_str().unwrap_or_default().to_string(),
                    name: o["name"].as_str().unwrap_or_default().to_string(),
                    kind: o["kind"].as_str().unwrap_or_default().to_string(),
                })
                .collect()
        })
        .unwrap_or_default();
    AgentEvent::Permission {
        id: request_id.to_string(),
        tool_call: params["toolCall"]["toolCallId"].as_str().unwrap_or_default().to_string(),
        options,
    }
}

/// What a reconnect emits when the agent cannot replay its own history.
pub fn load_unsupported_event() -> AgentEvent {
    AgentEvent::Gap { reason: AgentGapReason::LoadUnsupported }
}

pub struct AgentSession {
    conn: AcpConnection,
    pub session_id: String,
    pub available_modes: Vec<String>,
    pub available_commands: Vec<String>,
}

impl AgentSession {
    /// Initialize, then either load an existing session or create a new one.
    ///
    /// `resume` carries the session id from SQLite. Its absence means this is a
    /// terminal that has never been in agent pane mode.
    pub async fn start(
        mut conn: AcpConnection,
        resume: Option<String>,
    ) -> Result<(Self, Vec<AgentEvent>), SessionError> {
        let init = conn
            .request(
                "initialize",
                serde_json::json!({
                    "protocolVersion": 1,
                    "clientCapabilities": { "fs": { "readTextFile": true, "writeTextFile": true } }
                }),
            )
            .await?;
        let can_load = init["agentCapabilities"]["loadSession"].as_bool().unwrap_or(false);

        let mut prelude = Vec::new();
        let cwd = conn.worktree.display().to_string();

        let session_id = match resume {
            Some(id) if can_load => {
                conn.request(
                    "session/load",
                    serde_json::json!({ "sessionId": id, "cwd": cwd, "mcpServers": [] }),
                )
                .await?;
                id
            }
            Some(id) => {
                // Honest rather than convenient: the conversation continues, but
                // the history before this point cannot be shown.
                prelude.push(load_unsupported_event());
                conn.request(
                    "session/new",
                    serde_json::json!({ "cwd": cwd, "mcpServers": [] }),
                )
                .await?["sessionId"]
                    .as_str()
                    .unwrap_or(&id)
                    .to_string()
            }
            None => conn
                .request("session/new", serde_json::json!({ "cwd": cwd, "mcpServers": [] }))
                .await?["sessionId"]
                .as_str()
                .ok_or(SessionError::Rejected)?
                .to_string(),
        };

        let available_modes = init["agentCapabilities"]["availableModes"]
            .as_array()
            .map(|m| m.iter().filter_map(|v| v["id"].as_str().map(String::from)).collect())
            .unwrap_or_default();

        prelude.insert(
            0,
            AgentEvent::SessionStarted {
                session_id: session_id.clone(),
                agent_mode: init["agentCapabilities"]["currentModeId"].as_str().map(String::from),
                available_modes: available_modes.clone(),
                available_commands: Vec::new(),
            },
        );

        Ok((
            Self { conn, session_id, available_modes, available_commands: Vec::new() },
            prelude,
        ))
    }

    pub async fn prompt(&mut self, text: &str) -> Result<(), SessionError> {
        self.conn
            .notify(
                "session/prompt",
                serde_json::json!({
                    "sessionId": self.session_id,
                    "prompt": [{ "type": "text", "text": text }]
                }),
            )
            .await?;
        Ok(())
    }

    pub async fn answer(
        &mut self,
        request_id: serde_json::Value,
        option_id: &str,
    ) -> Result<(), SessionError> {
        self.conn
            .respond(
                request_id,
                serde_json::json!({ "outcome": { "outcome": "selected", "optionId": option_id } }),
            )
            .await?;
        Ok(())
    }

    pub async fn set_mode(&mut self, agent_mode: &str) -> Result<(), SessionError> {
        self.conn
            .notify(
                "session/set_mode",
                serde_json::json!({ "sessionId": self.session_id, "modeId": agent_mode }),
            )
            .await?;
        Ok(())
    }

    pub async fn cancel(&mut self) -> Result<(), SessionError> {
        self.conn
            .notify("session/cancel", serde_json::json!({ "sessionId": self.session_id }))
            .await?;
        Ok(())
    }

    /// Read one frame and turn it into events, answering capabilities inline.
    pub async fn pump(&mut self) -> Result<Vec<AgentEvent>, SessionError> {
        let Some(Incoming { id, method, params }) = self.conn.next_incoming().await? else {
            return Ok(Vec::new());
        };
        let worktree = self.conn.worktree.clone();
        match (method.as_str(), id) {
            ("fs/read_text_file", Some(id)) => {
                let path = params["path"].as_str().unwrap_or_default();
                match handle_fs_read(&worktree, path) {
                    Ok(content) => {
                        self.conn.respond(id, serde_json::json!({ "content": content })).await?
                    }
                    // Answered rather than left hanging: an unanswered request
                    // stalls the agent forever, and a refusal it can see is
                    // better than a turn that never ends.
                    Err(_) => self.conn.respond(id, serde_json::json!({ "content": "" })).await?,
                }
                Ok(Vec::new())
            }
            ("fs/write_text_file", Some(id)) => {
                let path = params["path"].as_str().unwrap_or_default();
                let content = params["content"].as_str().unwrap_or_default();
                let outcome = handle_fs_write(&worktree, path, content);
                self.conn.respond(id, serde_json::json!({})).await?;
                match outcome {
                    Ok(event) => Ok(vec![event]),
                    Err(_) => Ok(vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }]),
                }
            }
            ("session/request_permission", Some(id)) => {
                // The id is kept as JSON text so the client can hand back
                // exactly what the adapter sent; inventing a new id here would
                // make the answer unroutable.
                let request_id = serde_json::to_string(&id).unwrap_or_default();
                Ok(vec![permission_event(&request_id, &params)])
            }
            ("session/update", _) => {
                let rpc = Rpc { method: Some(method), params: Some(params), id: None, result: None };
                Ok(rpc
                    .session_notification()
                    .map(|n| update_to_events(&n.update))
                    .unwrap_or_else(|| vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }]))
            }
            _ => Ok(vec![AgentEvent::Gap { reason: AgentGapReason::Unparsed }]),
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-agent session`
Expected: PASS, 4 tests

- [ ] **Step 5: Commit**

```bash
git add crates/agent/src/session.rs crates/agent/src/lib.rs
git commit -m "feat(agent): a session that answers capabilities and reports diffs"
```

---

### Task 8: Activity derived from the protocol, not the screen

**Files:**
- Create: `crates/agent/src/activity_source.rs`
- Modify: `crates/agent/src/lib.rs`

**Interfaces:**
- Consumes: `event::AgentEvent` (Task 2), `farcooler_protocol::v1::AgentActivity`.
- Produces: `farcooler_agent::activity_source::observe(&AgentEvent) -> Option<AgentActivity>`.

This is the reliability win the spec names: in agent pane mode, `Blocked` stops being a generous list of hopeful substrings and becomes a protocol fact. `activity::advance`, `seen` and `wants_attention` in `crates/core` are reused **unchanged** — do not modify them.

- [ ] **Step 1: Write the failing test**

Create `crates/agent/src/activity_source.rs` containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AgentEvent, AgentGapReason, EndReason, PermissionOption, Role, ToolStatus};
    use farcooler_protocol::v1::AgentActivity;

    #[test]
    fn a_permission_request_is_exactly_blocked() {
        // The signal the whole notification story rests on. In agent pane mode
        // it is a protocol fact rather than a substring match.
        let e = AgentEvent::Permission {
            id: "r".into(),
            tool_call: "t".into(),
            options: vec![PermissionOption { id: "a".into(), name: "Yes".into(), kind: "allow_once".into() }],
        };
        assert_eq!(observe(&e), Some(AgentActivity::Blocked));
    }

    #[test]
    fn a_turn_ending_is_idle_so_advance_can_make_it_done() {
        // `advance(Working, Idle) == Done` lives in core and is not duplicated
        // here. This only has to report the observation honestly.
        assert_eq!(
            observe(&AgentEvent::TurnEnded { reason: EndReason::EndTurn }),
            Some(AgentActivity::Idle)
        );
    }

    #[test]
    fn work_in_flight_is_working() {
        assert_eq!(
            observe(&AgentEvent::Message { role: Role::Agent, text: "hi".into() }),
            Some(AgentActivity::Working)
        );
        assert_eq!(
            observe(&AgentEvent::ToolUpdate {
                id: "t".into(),
                status: ToolStatus::InProgress,
                content: None,
                diff: None
            }),
            Some(AgentActivity::Working)
        );
    }

    #[test]
    fn bookkeeping_events_do_not_move_the_badge() {
        // A gap or a mode change says nothing about whether the agent needs
        // you, and reporting one would make a fleet row flicker.
        assert_eq!(observe(&AgentEvent::Gap { reason: AgentGapReason::RingTrimmed }), None);
        assert_eq!(observe(&AgentEvent::ModeSet { agent_mode: "plan".into() }), None);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-agent activity_source`
Expected: FAIL — `cannot find function observe in this scope`

- [ ] **Step 3: Write the source**

Add `pub mod activity_source;` to `crates/agent/src/lib.rs`. Prepend to `crates/agent/src/activity_source.rs`:

```rust
//! What an agent is doing, read off the protocol instead of the screen.
//!
//! `core::activity` answers the same question for a TUI by matching substrings
//! on a captured pane, and has to be generous because a missed `blocked` is a
//! notification that never arrives. Here the agent says so itself.
//!
//! Only the OBSERVATION lives here. `advance`, `seen` and `wants_attention`
//! stay in `core` and are reused unchanged, so `Done` means the same thing and
//! a phone, a Mac badge and a notification cannot disagree.

use farcooler_protocol::v1::AgentActivity;

use crate::event::AgentEvent;

/// The activity this event implies, or `None` if it implies nothing.
pub fn observe(event: &AgentEvent) -> Option<AgentActivity> {
    match event {
        AgentEvent::Permission { .. } => Some(AgentActivity::Blocked),
        AgentEvent::TurnEnded { .. } => Some(AgentActivity::Idle),
        AgentEvent::Message { .. }
        | AgentEvent::ToolCall { .. }
        | AgentEvent::ToolUpdate { .. }
        | AgentEvent::Plan { .. }
        | AgentEvent::Resolved { .. } => Some(AgentActivity::Working),
        // Bookkeeping. Says nothing about whether the agent needs you.
        AgentEvent::SessionStarted { .. } | AgentEvent::ModeSet { .. } | AgentEvent::Gap { .. } => {
            None
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-agent activity_source`
Expected: PASS, 4 tests

- [ ] **Step 5: Commit**

```bash
git add crates/agent/src/activity_source.rs crates/agent/src/lib.rs
git commit -m "feat(agent): blocked becomes a fact instead of a hopeful substring"
```

---

### Task 9: The shim↔daemon socket protocol

**Files:**
- Create: `crates/agent/src/link.rs`
- Modify: `crates/agent/src/lib.rs`

**Interfaces:**
- Consumes: `event::{AgentEvent, Sequenced, Seq}` (Task 2), `ring::AgentReplay` (Task 4).
- Produces: `farcooler_agent::link::{ShimMessage, DaemonMessage, encode_line, decode_line}`.

Newline-delimited JSON, permitted by the Global Constraints because shim and daemon ship in one binary and therefore have no version skew. The client-facing protocol stays protobuf.

- [ ] **Step 1: Write the failing test**

Create `crates/agent/src/link.rs` containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{AgentEvent, Role, Sequenced};

    #[test]
    fn a_message_round_trips_through_one_line() {
        let msg = ShimMessage::Events {
            events: vec![Sequenced {
                seq: 3,
                event: AgentEvent::Message { role: Role::Agent, text: "hi".into() },
            }],
        };
        let line = encode_line(&msg).expect("encodes");
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1);
        let back: ShimMessage = decode_line(line.trim()).expect("decodes");
        assert_eq!(back, msg);
    }

    #[test]
    fn the_daemon_subscribes_from_a_cursor() {
        // Reconnect after a daemon restart is the whole reason this field
        // exists: the shim outlived the daemon and still holds the history.
        let line = encode_line(&DaemonMessage::Subscribe { from_seq: 12 }).expect("encodes");
        let back: DaemonMessage = decode_line(line.trim()).expect("decodes");
        assert_eq!(back, DaemonMessage::Subscribe { from_seq: 12 });
    }

    #[test]
    fn an_unknown_message_is_an_error_not_a_silent_drop() {
        assert!(decode_line::<DaemonMessage>(r#"{"kind":"from_the_future"}"#).is_err());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-agent link`
Expected: FAIL — `cannot find type ShimMessage in this scope`

- [ ] **Step 3: Write the link types**

Add `pub mod link;` to `crates/agent/src/lib.rs`. Add `serde` derives to the event model — in `crates/agent/src/event.rs`, change every `#[derive(Debug, Clone, PartialEq, Eq)]` to `#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]`.

Prepend to `crates/agent/src/link.rs`:

```rust
//! The private channel between a shim and its daemon.
//!
//! JSON lines rather than protobuf, and that is a deliberate exception to the
//! project's protocol rule rather than an oversight: the shim IS the daemon
//! binary under another subcommand, so the two can never disagree about a
//! schema. There is no compatibility surface here to protect.

use serde::{Deserialize, Serialize};

use crate::event::{Seq, Sequenced};

/// Shim to daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShimMessage {
    /// Events at and after the subscribed cursor.
    Events { events: Vec<Sequenced> },
    /// The subscriber's cursor had been trimmed. A `Gap` is already the first
    /// entry in `events`; this carries the accounting for logs.
    Trimmed { resumed_at: Seq, dropped: u64, events: Vec<Sequenced> },
    /// The session is established and this is its id, for durable intent.
    Established { session_id: String, available_modes: Vec<String> },
    /// The adapter could not be started. Terminal-mode fallback remains.
    Failed { reason: String },
}

/// Daemon to shim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DaemonMessage {
    Subscribe { from_seq: Seq },
    Prompt { text: String },
    Answer { request_id: String, option_id: String },
    SetMode { agent_mode: String },
    Cancel,
}

pub fn encode_line<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    Ok(format!("{}\n", serde_json::to_string(value)?))
}

pub fn decode_line<T: for<'de> Deserialize<'de>>(line: &str) -> Result<T, serde_json::Error> {
    serde_json::from_str(line)
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-agent link`
Expected: PASS, 3 tests

Then confirm nothing else broke: `cargo test -p farcooler-agent`

- [ ] **Step 5: Commit**

```bash
git add crates/agent/src/link.rs crates/agent/src/event.rs crates/agent/src/lib.rs
git commit -m "feat(agent): a private channel between a shim and its daemon"
```

---

### Task 10: `farcooler agent-host` — the shim that lives in the pane

**Files:**
- Create: `crates/cli/src/agent_host.rs`
- Modify: `crates/cli/src/main.rs`
- Modify: `crates/cli/Cargo.toml`

**Interfaces:**
- Consumes: `farcooler_agent::{session::AgentSession, acp::conn::AcpConnection, ring::AgentRing, link::{ShimMessage, DaemonMessage, encode_line, decode_line}, ring::AgentReplay}` (Tasks 4, 6, 7, 9).
- Produces: the subcommand `farcooler agent-host --terminal <uuid> --socket <path> --worktree <path> [--session <uuid>] [--adapter <program>]`.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/cli/src/agent_host.rs` (created in step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_line_is_readable_by_a_person_looking_at_the_pane() {
        // The pane is a real pane and the user can attach to it. When the
        // adapter cannot start, what is written here is the entire error
        // message they get, so it has to stand alone.
        let line = status_line(&Status::AdapterMissing { program: "claude-code-acp".into() });
        assert!(line.contains("claude-code-acp"));
        assert!(line.contains("terminal mode"), "must name the working fallback: {line}");
    }

    #[test]
    fn the_default_adapter_is_the_zed_package() {
        let (program, args) = default_adapter();
        assert_eq!(program, "npx");
        assert!(args.iter().any(|a| a.contains("@zed-industries/claude-code-acp")));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-cli agent_host`
Expected: FAIL — `file not found for module agent_host`

- [ ] **Step 3: Write the shim**

Add to `crates/cli/Cargo.toml` `[dependencies]`:

```toml
farcooler-agent.workspace = true
```

Add `mod agent_host;` to `crates/cli/src/main.rs`, and a variant to its `clap` command enum:

```rust
    /// Host a headless coding agent in this pane. Started by the daemon.
    ///
    /// Not a command a user types. It is the process a pane runs in agent pane
    /// mode, and it is a subcommand rather than a second binary so that shim
    /// and daemon can never be different versions.
    AgentHost {
        #[arg(long)]
        terminal: uuid::Uuid,
        #[arg(long)]
        socket: std::path::PathBuf,
        #[arg(long)]
        worktree: std::path::PathBuf,
        #[arg(long)]
        session: Option<String>,
        #[arg(long)]
        adapter: Option<String>,
    },
```

Dispatch it in the same `match` that handles the other commands:

```rust
        Command::AgentHost { terminal, socket, worktree, session, adapter } => {
            agent_host::run(terminal, socket, worktree, session, adapter).await
        }
```

Prepend to `crates/cli/src/agent_host.rs`:

```rust
//! The process a pane runs in agent pane mode.
//!
//! It exists so that a headless agent still has a tagged tmux pane, which keeps
//! `derive.rs` the single authority on whether anything is alive. It also owns
//! the event ring, because it is the process that lives exactly as long as the
//! pane does — a daemon restart therefore costs no history.
//!
//! What it prints to its own stdout is not decoration. The pane is real and
//! attachable, so this log is what a user sees when they go looking.

use std::path::PathBuf;

use farcooler_agent::acp::conn::AcpConnection;
use farcooler_agent::link::{DaemonMessage, ShimMessage, decode_line, encode_line};
use farcooler_agent::ring::{AgentReplay, AgentRing};
use farcooler_agent::session::AgentSession;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use uuid::Uuid;

pub enum Status {
    AdapterMissing { program: String },
    Connected { session_id: String },
}

pub fn status_line(status: &Status) -> String {
    match status {
        Status::AdapterMissing { program } => format!(
            "farcooler: could not start the ACP adapter `{program}`.\n\
             Install it, or switch this terminal back to terminal mode — \
             terminal mode needs no adapter and is unaffected."
        ),
        Status::Connected { session_id } => {
            format!("farcooler: agent session {session_id} connected. Rendering natively.")
        }
    }
}

/// The adapter Far Cooler uses when preferences name none.
pub fn default_adapter() -> (String, Vec<String>) {
    (
        "npx".to_string(),
        vec!["-y".to_string(), "@zed-industries/claude-code-acp".to_string()],
    )
}

pub async fn run(
    terminal: Uuid,
    socket: PathBuf,
    worktree: PathBuf,
    session: Option<String>,
    adapter: Option<String>,
) -> anyhow::Result<()> {
    let (program, args) = match adapter {
        Some(a) => (a, Vec::new()),
        None => default_adapter(),
    };

    let conn = match AcpConnection::spawn(&program, &args, &worktree).await {
        Ok(c) => c,
        Err(_) => {
            println!("{}", status_line(&Status::AdapterMissing { program }));
            // Stay alive so the pane does not vanish and derive as an exit the
            // user never caused. They read the message and switch modes.
            std::future::pending::<()>().await;
            unreachable!()
        }
    };

    let (mut agent, prelude) = AgentSession::start(conn, session).await?;
    println!("{}", status_line(&Status::Connected { session_id: agent.session_id.clone() }));

    let mut ring = AgentRing::new();
    for event in prelude {
        ring.push(event);
    }

    // The daemon may restart under us. Reconnect forever; the ring is what
    // makes that free.
    loop {
        let Ok(stream) = UnixStream::connect(&socket).await else {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            continue;
        };
        if let Err(e) = serve(&mut agent, &mut ring, stream, terminal).await {
            tracing::warn!(terminal = %terminal, error = %e, "daemon link dropped; will reconnect");
        }
    }
}

async fn serve(
    agent: &mut AgentSession,
    ring: &mut AgentRing,
    stream: UnixStream,
    terminal: Uuid,
) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    let mut lines = BufReader::new(read_half).lines();

    write_half
        .write_all(
            encode_line(&ShimMessage::Established {
                session_id: agent.session_id.clone(),
                available_modes: agent.available_modes.clone(),
            })?
            .as_bytes(),
        )
        .await?;

    let mut cursor = 0u64;

    loop {
        tokio::select! {
            line = lines.next_line() => {
                let Some(line) = line? else { return Ok(()) };
                match decode_line::<DaemonMessage>(&line)? {
                    DaemonMessage::Subscribe { from_seq } => {
                        cursor = from_seq;
                        let message = match ring.since(from_seq) {
                            AgentReplay::At { events } => ShimMessage::Events { events },
                            AgentReplay::Gap { resumed_at, dropped, events } =>
                                ShimMessage::Trimmed { resumed_at, dropped, events },
                        };
                        cursor = ring.next_seq();
                        write_half.write_all(encode_line(&message)?.as_bytes()).await?;
                    }
                    DaemonMessage::Prompt { text } => agent.prompt(&text).await?,
                    DaemonMessage::Answer { request_id, option_id } => {
                        let id: serde_json::Value = serde_json::from_str(&request_id)
                            .unwrap_or(serde_json::Value::String(request_id.clone()));
                        agent.answer(id, &option_id).await?;
                    }
                    DaemonMessage::SetMode { agent_mode } => agent.set_mode(&agent_mode).await?,
                    DaemonMessage::Cancel => agent.cancel().await?,
                }
            }
            pumped = agent.pump() => {
                let events = pumped?;
                if events.is_empty() { continue }
                let mut batch = Vec::with_capacity(events.len());
                for event in events {
                    let seq = ring.push(event.clone());
                    batch.push(farcooler_agent::event::Sequenced { seq, event });
                }
                cursor = ring.next_seq();
                let _ = terminal;
                write_half
                    .write_all(encode_line(&ShimMessage::Events { events: batch })?.as_bytes())
                    .await?;
            }
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-cli agent_host`
Expected: PASS, 2 tests

Then: `cargo build --workspace` — expected to succeed.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/agent_host.rs crates/cli/src/main.rs crates/cli/Cargo.toml Cargo.lock
git commit -m "feat(cli): agent-host, so a headless agent still has a pane"
```

---

### Task 11: Durable intent — `pane_mode` and `agent_session_id`

**Files:**
- Modify: `crates/store/src/migrate.rs`
- Modify: `crates/store/src/models.rs`
- Modify: `crates/store/src/store.rs`

**Interfaces:**
- Produces: `models::PaneMode { Terminal, Agent }`, `models::Terminal.pane_mode`, `models::Terminal.agent_session_id: Option<String>`, and `Store::set_pane_mode(id, expected_version, pane_mode, agent_session_id) -> Result<Terminal>`.

- [ ] **Step 1: Write the failing test**

Add to the bottom of `crates/store/src/store.rs` inside its existing `#[cfg(test)] mod tests` block (create the block if absent, using the crate's existing test helpers to open a temporary store):

```rust
    #[test]
    fn a_new_terminal_starts_in_terminal_pane_mode() {
        // Terminal-first is the product's default, and defaults belong in the
        // schema rather than in whichever caller remembered.
        let store = temp_store();
        let ws = seed_workspace(&store);
        let t = store.create_terminal(ws, "t", "claude", TerminalIntent::Running, 120, 40).unwrap();
        assert_eq!(t.pane_mode, PaneMode::Terminal);
        assert_eq!(t.agent_session_id, None);
    }

    #[test]
    fn a_session_id_survives_a_reopen_because_it_is_intent_not_runtime() {
        // The one thing about an agent session that must outlive tmux. The
        // conversation itself is never stored.
        let store = temp_store();
        let ws = seed_workspace(&store);
        let t = store.create_terminal(ws, "t", "claude", TerminalIntent::Running, 120, 40).unwrap();
        let updated = store
            .set_pane_mode(t.id, t.resource_version, PaneMode::Agent, Some("abc-123".into()))
            .unwrap();
        assert_eq!(updated.pane_mode, PaneMode::Agent);

        let reopened = store.get_terminal(t.id).unwrap();
        assert_eq!(reopened.agent_session_id.as_deref(), Some("abc-123"));
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-store pane_mode`
Expected: FAIL — `no field pane_mode on type Terminal`

- [ ] **Step 3: Add the migration, the model fields and the mutator**

In `crates/store/src/migrate.rs`, append to `MIGRATIONS`:

```rust
    migration_0004_pane_mode,
```

and add the migration function:

```rust
/// Agent pane mode, and the session id that outlives every pane hosting it.
///
/// `agent_session_id` is intent, in the same sense as the branch: it says what
/// this terminal is FOR. The conversation it names is never stored here — the
/// shim holds that in memory and says `Gap` where it cannot account for it.
fn migration_0004_pane_mode(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        r#"
        ALTER TABLE terminals ADD COLUMN pane_mode INTEGER NOT NULL DEFAULT 0;
        ALTER TABLE terminals ADD COLUMN agent_session_id TEXT;
        "#,
    )
}
```

In `crates/store/src/models.rs`, add the enum and the two fields on `Terminal`:

```rust
/// What a terminal's pane is hosting.
///
/// Distinct from a terminal's VT `mode` and from the ACP `agent_mode`. Three
/// unrelated things would otherwise all be called "mode" in one pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaneMode {
    /// A TUI, exactly as before this feature existed. The default.
    Terminal,
    /// `farcooler agent-host`, bridging an ACP agent.
    Agent,
}

impl PaneMode {
    pub fn as_i64(self) -> i64 {
        match self {
            PaneMode::Terminal => 0,
            PaneMode::Agent => 1,
        }
    }

    pub fn from_i64(raw: i64) -> Self {
        match raw {
            1 => PaneMode::Agent,
            // Anything unrecognized is the mode that always works.
            _ => PaneMode::Terminal,
        }
    }
}
```

Add to `struct Terminal`:

```rust
    pub pane_mode: PaneMode,
    pub agent_session_id: Option<String>,
```

In `crates/store/src/store.rs`, three `SELECT` statements build a `Terminal` — in `get_terminal`, `list_terminals_for_workspace` and `load_terminal_records`. Append `pane_mode, agent_session_id` to each column list, and add the two fields to each row mapper at the corresponding new indices:

```rust
            pane_mode: models::PaneMode::from_i64(row.get(13)?),
            agent_session_id: row.get(14)?,
```

Adjust those indices to sit after the existing columns in each statement — `resource_version` is currently last, so the two new ones follow it. In `create_terminal`'s constructed `Terminal` (around line 391), add:

```rust
            pane_mode: PaneMode::Terminal,
            agent_session_id: None,
```

Then add the mutator:

```rust
    /// Record which mode this terminal's pane is in, and the session it names.
    ///
    /// Version-checked like every other mutation: two clients toggling the same
    /// pane must not both believe they won.
    ///
    /// A session id is never CLEARED by a mode change. Switching to terminal
    /// mode and back has to land on the same conversation, so `None` means
    /// "leave it alone" rather than "forget it".
    pub fn set_pane_mode(
        &self,
        id: Uuid,
        expected_version: u64,
        pane_mode: models::PaneMode,
        agent_session_id: Option<String>,
    ) -> Result<models::Terminal> {
        let changed = self
            .conn()
            .execute(
                r#"UPDATE terminals
                      SET pane_mode = ?1,
                          agent_session_id = COALESCE(?2, agent_session_id),
                          resource_version = resource_version + 1
                    WHERE id = ?3 AND resource_version = ?4"#,
                rusqlite::params![
                    pane_mode.as_i64(),
                    agent_session_id,
                    id.as_bytes().as_slice(),
                    expected_version as i64,
                ],
            )
            .map_err(map_err)?;

        if changed == 0 {
            // Either the terminal is gone or someone else moved it first. Both
            // are the caller's problem to re-read, not ours to paper over.
            return Err(DomainError::ResourceConflict);
        }
        self.get_terminal(id)
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-store`
Expected: PASS, including the two new tests

- [ ] **Step 5: Commit**

```bash
git add crates/store/
git commit -m "feat(store): pane mode and a session id, as intent and nothing more"
```

---

### Task 12: Declare the session id at launch

**Files:**
- Modify: `crates/daemon/src/service.rs:36-59` (`preset_command`)
- Modify: `crates/daemon/src/service.rs:562` (`create_terminal`) and its other call sites

**Interfaces:**
- Produces: `preset_command(preset: &str, session_id: Option<&str>) -> String`.

Discovery is archaeology. Declaring is exact. Every claude terminal Far Cooler launches gets a uuid Far Cooler chose, so adoption later is a lookup rather than a guess.

- [ ] **Step 1: Write the failing test**

Add to `crates/daemon/src/service.rs`'s test module:

```rust
    #[test]
    fn a_claude_terminal_is_launched_with_the_session_id_we_chose() {
        // So that switching this pane to agent mode later is a lookup rather
        // than a guess about which of several .jsonl files is ours.
        let cmd = preset_command("claude", Some("018f5b2c-0000-7000-8000-000000000000"));
        assert!(cmd.contains("--session-id 018f5b2c-0000-7000-8000-000000000000"), "{cmd}");
    }

    #[test]
    fn a_shell_is_not_given_a_session_id() {
        let cmd = preset_command("shell", Some("018f5b2c-0000-7000-8000-000000000000"));
        assert!(!cmd.contains("--session-id"), "{cmd}");
    }

    #[test]
    fn a_session_id_that_is_not_a_uuid_is_dropped_rather_than_escaped() {
        // It ends up inside a `-ilc` string. The existing rule for models
        // applies here for the same reason.
        let cmd = preset_command("claude", Some("; rm -rf /"));
        assert!(!cmd.contains("rm -rf"), "{cmd}");
        assert!(!cmd.contains("--session-id"), "{cmd}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-daemon preset_command`
Expected: FAIL — `this function takes 1 argument but 2 arguments were supplied`

- [ ] **Step 3: Thread the session id through**

Replace the body of `preset_command` in `crates/daemon/src/service.rs`:

```rust
pub fn preset_command(preset: &str, session_id: Option<&str>) -> String {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
    let (agent, model) = match preset.split_once(':') {
        Some((a, m)) if is_safe_model(m) => (a, Some(m)),
        Some((a, _)) => (a, None),
        None => (preset, None),
    };

    let flag = model.map(|m| format!(" --model {m}")).unwrap_or_default();

    // Declared, not discovered — but it still ends up inside a `-ilc` string,
    // so anything that is not a plain uuid is dropped rather than escaped. The
    // cost of dropping it is one adoption that has to fall back to searching.
    let session = session_id
        .filter(|s| Uuid::parse_str(s).is_ok())
        .map(|s| format!(" --session-id {s}"))
        .unwrap_or_default();

    match agent {
        "shell" => format!("{shell} -il"),
        "claude" => format!("{shell} -ilc 'claude{flag}{session}'"),
        "codex" => format!("{shell} -ilc 'codex{flag}'"),
        "cursor" => format!("{shell} -ilc 'cursor-agent{flag}'"),
        other if is_safe_model(other) => format!("{shell} -ilc '{other}{flag}'"),
        _ => format!("{shell} -il"),
    }
}
```

In `create_terminal`, generate and persist the id before creating the window. After the `self.store.create_terminal(...)` call and before `let command = ...`:

```rust
        // A claude terminal gets its session id now, so that adopting it into
        // agent pane mode later is exact.
        let declared = command_preset.starts_with("claude").then(|| Uuid::now_v7().to_string());
        let term = if let Some(ref sid) = declared {
            self.store.set_pane_mode(
                term.id,
                term.resource_version,
                models::PaneMode::Terminal,
                Some(sid.clone()),
            )?
        } else {
            term
        };

        let command = preset_command(command_preset, declared.as_deref());
```

Update the other two call sites (`split_terminal` and `restart_terminal`) to pass the terminal's stored `agent_session_id.as_deref()`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-daemon`
Expected: PASS, including the three new tests

- [ ] **Step 5: Commit**

```bash
git add crates/daemon/src/service.rs
git commit -m "feat(daemon): a claude session is declared at launch, not excavated later"
```

---

### Task 13: Find a hand-started agent's session, or refuse

**Files:**
- Create: `crates/daemon/src/session_discovery.rs`
- Modify: `crates/daemon/src/lib.rs`

**Interfaces:**
- Produces: `session_discovery::{project_dir_name, discover_claude_session, DiscoveryError}` with `discover_claude_session(home: &Path, worktree: &Path, started_after: SystemTime) -> Result<String, DiscoveryError>`.

The fallback path for `claude` typed into a shell by hand. Attaching to the wrong conversation is worse than not offering, so ambiguity refuses.

- [ ] **Step 1: Write the failing test**

Create `crates/daemon/src/session_discovery.rs` containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    #[test]
    fn a_worktree_path_munges_the_way_claude_munges_it() {
        // Read off a real machine: every non-alphanumeric becomes a dash, and
        // the leading slash produces a leading dash.
        assert_eq!(project_dir_name(Path::new("/Users/e/Dev/farcooler")), "-Users-e-Dev-farcooler");
        assert_eq!(project_dir_name(Path::new("/Users/e/.claude/jobs")), "-Users-e--claude-jobs");
    }

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("farcooler-disc-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn one_session_in_the_worktree_is_found() {
        let home = scratch("one");
        let worktree = Path::new("/Users/e/Dev/proj");
        let dir = home.join(".claude/projects").join(project_dir_name(worktree));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sess-a.jsonl"), "{}").unwrap();

        let found = discover_claude_session(&home, worktree, SystemTime::UNIX_EPOCH).unwrap();
        assert_eq!(found, "sess-a");
    }

    #[test]
    fn two_candidate_sessions_refuse_rather_than_pick_one() {
        // The failure this rule prevents is silent and expensive: attaching a
        // chat view to a conversation that is not the one in the pane.
        let home = scratch("two");
        let worktree = Path::new("/Users/e/Dev/proj2");
        let dir = home.join(".claude/projects").join(project_dir_name(worktree));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("sess-a.jsonl"), "{}").unwrap();
        std::fs::write(dir.join("sess-b.jsonl"), "{}").unwrap();

        let err = discover_claude_session(&home, worktree, SystemTime::UNIX_EPOCH).unwrap_err();
        let DiscoveryError::Ambiguous { candidates } = err else { panic!("expected refusal") };
        assert_eq!(candidates.len(), 2);
    }

    #[test]
    fn a_session_older_than_the_pane_is_not_a_candidate() {
        // A worktree reused for a second task holds a stale session file. It
        // predates this pane and is therefore not what is running in it.
        let home = scratch("stale");
        let worktree = Path::new("/Users/e/Dev/proj3");
        let dir = home.join(".claude/projects").join(project_dir_name(worktree));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("old.jsonl"), "{}").unwrap();

        let future = SystemTime::now() + Duration::from_secs(3600);
        assert!(matches!(
            discover_claude_session(&home, worktree, future),
            Err(DiscoveryError::NotFound)
        ));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-daemon session_discovery`
Expected: FAIL — `file not found for module session_discovery`

- [ ] **Step 3: Write the discovery**

Add `pub mod session_discovery;` to `crates/daemon/src/lib.rs`. Prepend to `crates/daemon/src/session_discovery.rs`:

```rust
//! Which conversation is running in a pane nobody declared.
//!
//! Only for `claude` typed into a shell by hand. Anything Far Cooler launched
//! carries a declared session id in SQLite and never reaches here.
//!
//! A workspace is one worktree, so the project directory almost always holds a
//! single session and this is unambiguous. When it is not, this refuses and
//! names what it found. Guessing would attach a chat view to the wrong
//! conversation, which is a silent, expensive kind of wrong — the same reason
//! an unproven terminal derives as `LOST` rather than as a plausible state.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    #[error("no session for this worktree")]
    NotFound,
    #[error("more than one session could be the one in this pane")]
    Ambiguous { candidates: Vec<String> },
}

/// Claude Code's project-directory name for a working directory.
///
/// Every non-alphanumeric character becomes a dash, which is why an absolute
/// path gains a leading one and a dotfile directory gains a double.
pub fn project_dir_name(worktree: &Path) -> String {
    worktree
        .to_string_lossy()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The session id for a hand-started claude in `worktree`.
///
/// `started_after` is the pane's start time: a session file older than the pane
/// cannot be the one running in it.
pub fn discover_claude_session(
    home: &Path,
    worktree: &Path,
    started_after: SystemTime,
) -> Result<String, DiscoveryError> {
    let dir: PathBuf = home.join(".claude/projects").join(project_dir_name(worktree));
    let entries = std::fs::read_dir(&dir).map_err(|_| DiscoveryError::NotFound)?;

    let mut candidates: Vec<String> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let modified = entry.metadata().and_then(|m| m.modified()).ok();
        if modified.map(|m| m < started_after).unwrap_or(true) {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            candidates.push(stem.to_string());
        }
    }

    match candidates.len() {
        0 => Err(DiscoveryError::NotFound),
        1 => Ok(candidates.remove(0)),
        _ => {
            candidates.sort();
            Err(DiscoveryError::Ambiguous { candidates })
        }
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-daemon session_discovery`
Expected: PASS, 4 tests

- [ ] **Step 5: Commit**

```bash
git add crates/daemon/src/session_discovery.rs crates/daemon/src/lib.rs
git commit -m "feat(daemon): find a hand-started session, or say which ones it could be"
```

---

### Task 14: `respawn-pane`, so the toggle keeps its place in the layout

**Files:**
- Modify: `crates/tmux/src/windows.rs`
- Modify: `crates/tmux/tests/live_tmux.rs`

**Interfaces:**
- Produces: `TmuxServer::respawn_pane(&self, pane_id: &str, worktree: &str, command: &str) -> Result<()>`.

Kill-and-create would drop the pane out of its position, which is exactly wrong when a chat is one tile of four. `respawn-pane -k` replaces the process in place, so pane id, tag, rectangle and layout all survive.

- [ ] **Step 1: Write the failing test**

Add to `crates/tmux/tests/live_tmux.rs`, following the file's existing pattern for gating on a live tmux:

```rust
#[tokio::test]
async fn respawning_a_pane_keeps_its_id_its_tag_and_its_place() {
    // The toggle's whole correctness argument. If the pane id changed, the
    // terminal would become unidentifiable and derive as `lost`; if the
    // rectangle changed, a four-tile layout would reflow every time someone
    // opened a chat.
    let Some(server) = live_server().await else { return };
    let workspace = Uuid::now_v7();
    let terminal = Uuid::now_v7();
    let window = server
        .create_terminal_window(workspace, terminal, "respawn", "/tmp", "/bin/sh -c 'sleep 300'")
        .await
        .expect("window");

    let before = server.list_tagged_panes().await.expect("panes");
    let pane = before.iter().find(|p| p.terminal_id == terminal).expect("tagged pane");
    let pane_id = pane.pane_id.clone();

    server
        .respawn_pane(&pane_id, "/tmp", "/bin/sh -c 'sleep 300'")
        .await
        .expect("respawn succeeds");

    let after = server.list_tagged_panes().await.expect("panes");
    let same = after.iter().find(|p| p.terminal_id == terminal).expect("still tagged");
    assert_eq!(same.pane_id, pane_id, "pane identity must survive a respawn");

    let _ = server.kill_terminal_window(terminal).await;
    let _ = window;
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-tmux --test live_tmux respawning`
Expected: FAIL — `no method named respawn_pane`

- [ ] **Step 3: Add the method**

Add to `impl TmuxServer` in `crates/tmux/src/windows.rs`, beside `kill_pane`:

```rust
    /// Replace the process in a pane, keeping the pane.
    ///
    /// This is how pane mode is toggled. `kill-pane` plus `new-window` would
    /// give the terminal a new pane id — losing its tag, and its position in
    /// whatever layout the user had built — so a chat opening in one tile of
    /// four would rearrange the other three.
    ///
    /// `-k` kills whatever is running first; without it tmux refuses on a live
    /// pane. The working directory goes through tmux's validated `-c` rather
    /// than as `cd` text, for the same reason as `create_terminal_window`.
    pub async fn respawn_pane(&self, pane_id: &str, worktree: &str, command: &str) -> Result<()> {
        let out = self
            .run(&["respawn-pane", "-k", "-t", pane_id, "-c", worktree, command])
            .await?;
        if !out.ok() {
            tracing::warn!(pane = %pane_id, stderr = %out.stderr, "respawn-pane failed");
            return Err(DomainError::TmuxUnavailable);
        }
        Ok(())
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-tmux --test live_tmux respawning`
Expected: PASS (or a clean skip if the harness finds no live tmux — verify by running with tmux available)

- [ ] **Step 5: Commit**

```bash
git add crates/tmux/
git commit -m "feat(tmux): respawn a pane so a toggle keeps its place in the layout"
```

---

### Task 15: The daemon's agent supervisor

**Files:**
- Create: `crates/daemon/src/agent_supervisor.rs`
- Modify: `crates/daemon/src/lib.rs`
- Modify: `crates/daemon/src/service.rs`

**Interfaces:**
- Consumes: `farcooler_agent::link::{ShimMessage, DaemonMessage}` (Task 9), `TmuxServer::respawn_pane` (Task 14), `Store::set_pane_mode` (Task 11), `session_discovery` (Task 13), `activity_source::observe` (Task 8), `core::activity::advance`.
- Produces: `AgentSupervisor::{socket_path, listen, set_pane_mode, prompt, answer, set_agent_mode, cancel, subscribe}` and `Service::set_pane_mode(terminal_id, PaneMode, force: bool) -> Result<models::Terminal>`.

- [ ] **Step 1: Write the failing test**

Create `crates/daemon/src/agent_supervisor.rs` containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_agent::event::{AgentEvent, EndReason, PermissionOption, Role};
    use farcooler_protocol::v1::AgentActivity;

    #[test]
    fn a_socket_path_is_per_terminal_and_not_guessable_across_daemons() {
        let a = socket_path(Path::new("/run/farcooler"), Uuid::now_v7());
        let b = socket_path(Path::new("/run/farcooler"), Uuid::now_v7());
        assert_ne!(a, b);
        assert!(a.starts_with("/run/farcooler"));
    }

    #[test]
    fn activity_folds_through_core_so_done_still_means_unseen() {
        // The rule that makes a notification worth sending lives in core and is
        // not reimplemented here. Working -> Idle is what produces Done.
        let mut current = AgentActivity::Unspecified;
        current = fold_activity(current, &AgentEvent::Message { role: Role::Agent, text: "x".into() });
        assert_eq!(current, AgentActivity::Working);
        current = fold_activity(current, &AgentEvent::TurnEnded { reason: EndReason::EndTurn });
        assert_eq!(current, AgentActivity::Done);
    }

    #[test]
    fn a_permission_request_blocks_the_row_immediately() {
        let e = AgentEvent::Permission {
            id: "r".into(),
            tool_call: "t".into(),
            options: vec![PermissionOption { id: "a".into(), name: "Yes".into(), kind: "allow_once".into() }],
        };
        assert_eq!(fold_activity(AgentActivity::Working, &e), AgentActivity::Blocked);
    }

    #[test]
    fn switching_to_terminal_mode_mid_turn_is_refused_unless_forced() {
        // `claude --resume` cannot attach to a turn in flight, so a quiet
        // switch would discard work the user is watching.
        assert!(matches!(
            guard_toggle(AgentActivity::Working, false),
            Err(ToggleRefusal::TurnInFlight)
        ));
        assert!(guard_toggle(AgentActivity::Working, true).is_ok());
        assert!(guard_toggle(AgentActivity::Idle, false).is_ok());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-daemon agent_supervisor`
Expected: FAIL — `file not found for module agent_supervisor`

- [ ] **Step 3: Write the supervisor**

Add `pub mod agent_supervisor;` to `crates/daemon/src/lib.rs`. Prepend to `crates/daemon/src/agent_supervisor.rs`:

```rust
//! The daemon's half of every agent session.
//!
//! It owns no transcript. The shim holds the ring, because the shim lives
//! exactly as long as the pane whose liveness is already authoritative — so a
//! daemon restart costs no history and needs no `session/load`.
//!
//! What lives here is the bookkeeping only the daemon can do: which terminals
//! are in agent pane mode, what each one's activity is, and fanning events out
//! to however many clients are watching.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use farcooler_agent::event::AgentEvent;
use farcooler_agent::link::{DaemonMessage, ShimMessage, decode_line, encode_line};
use farcooler_agent::{activity_source, event::Seq};
use farcooler_core::activity;
use farcooler_protocol::v1::AgentActivity;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use uuid::Uuid;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ToggleRefusal {
    #[error("a turn is in flight; cancel it or force the switch")]
    TurnInFlight,
}

/// Where a terminal's shim dials.
///
/// Per terminal and per runtime directory, so two daemons on one host never
/// collide and a stale socket never adopts a new session.
pub fn socket_path(runtime_dir: &Path, terminal: Uuid) -> PathBuf {
    runtime_dir.join(format!("agent-{terminal}.sock"))
}

/// Apply one event to a terminal's activity.
///
/// The observation is `farcooler_agent`'s; the FOLD is `core::activity`'s, and
/// deliberately so. `Done` must mean the same thing whether it came from a
/// screen or from a protocol, or a Mac badge and a phone notification will
/// disagree about the same terminal.
pub fn fold_activity(current: AgentActivity, event: &AgentEvent) -> AgentActivity {
    match activity_source::observe(event) {
        Some(observed) => activity::advance(current, observed),
        None => current,
    }
}

/// Whether a pane-mode toggle may proceed.
pub fn guard_toggle(current: AgentActivity, force: bool) -> Result<(), ToggleRefusal> {
    if force {
        return Ok(());
    }
    match current {
        AgentActivity::Working => Err(ToggleRefusal::TurnInFlight),
        _ => Ok(()),
    }
}

#[derive(Debug, Default)]
struct SessionState {
    activity: AgentActivity,
    cursor: Seq,
    session_id: Option<String>,
    available_modes: Vec<String>,
}

#[derive(Clone, Default)]
pub struct AgentSupervisor {
    sessions: Arc<Mutex<HashMap<Uuid, SessionState>>>,
    writers: Arc<Mutex<HashMap<Uuid, tokio::sync::mpsc::UnboundedSender<DaemonMessage>>>>,
}

impl AgentSupervisor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn activity(&self, terminal: Uuid) -> AgentActivity {
        self.sessions
            .lock()
            .ok()
            .and_then(|s| s.get(&terminal).map(|st| st.activity))
            .unwrap_or(AgentActivity::Unspecified)
    }

    pub fn session_id(&self, terminal: Uuid) -> Option<String> {
        self.sessions.lock().ok().and_then(|s| s.get(&terminal).and_then(|st| st.session_id.clone()))
    }

    pub fn send(&self, terminal: Uuid, message: DaemonMessage) {
        if let Ok(writers) = self.writers.lock() {
            if let Some(tx) = writers.get(&terminal) {
                let _ = tx.send(message);
            }
        }
    }

    /// Accept the shim for one terminal and pump it until the pane dies.
    ///
    /// `on_events` is how the daemon fans out; it is a callback rather than a
    /// channel so that the existing event bus stays the only fanout in the
    /// process.
    pub async fn listen<F>(
        &self,
        runtime_dir: &Path,
        terminal: Uuid,
        on_events: F,
    ) -> std::io::Result<()>
    where
        F: Fn(Uuid, Vec<farcooler_agent::event::Sequenced>) + Send + 'static,
    {
        let path = socket_path(runtime_dir, terminal);
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;

        loop {
            let (stream, _) = listener.accept().await?;
            let cursor = self
                .sessions
                .lock()
                .ok()
                .and_then(|s| s.get(&terminal).map(|st| st.cursor))
                .unwrap_or(0);
            if let Err(e) = self.serve(stream, terminal, cursor, &on_events).await {
                tracing::warn!(terminal = %terminal, error = %e, "agent shim link ended");
            }
        }
    }

    async fn serve<F>(
        &self,
        stream: UnixStream,
        terminal: Uuid,
        cursor: Seq,
        on_events: &F,
    ) -> std::io::Result<()>
    where
        F: Fn(Uuid, Vec<farcooler_agent::event::Sequenced>) + Send + 'static,
    {
        let (read_half, mut write_half) = stream.into_split();
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DaemonMessage>();
        if let Ok(mut writers) = self.writers.lock() {
            writers.insert(terminal, tx);
        }

        let subscribe = encode_line(&DaemonMessage::Subscribe { from_seq: cursor })
            .unwrap_or_else(|_| "\n".to_string());
        write_half.write_all(subscribe.as_bytes()).await?;

        let mut lines = BufReader::new(read_half).lines();
        loop {
            tokio::select! {
                outgoing = rx.recv() => {
                    let Some(message) = outgoing else { return Ok(()) };
                    if let Ok(line) = encode_line(&message) {
                        write_half.write_all(line.as_bytes()).await?;
                    }
                }
                line = lines.next_line() => {
                    let Some(line) = line? else { return Ok(()) };
                    let Ok(message) = decode_line::<ShimMessage>(&line) else { continue };
                    self.apply(terminal, message, on_events);
                }
            }
        }
    }

    fn apply<F>(&self, terminal: Uuid, message: ShimMessage, on_events: &F)
    where
        F: Fn(Uuid, Vec<farcooler_agent::event::Sequenced>) + Send + 'static,
    {
        let batch = match message {
            ShimMessage::Events { events } => events,
            // The gap is already the first entry; the counters are for logs.
            ShimMessage::Trimmed { resumed_at, dropped, events } => {
                tracing::info!(terminal = %terminal, resumed_at, dropped, "agent ring trimmed");
                events
            }
            ShimMessage::Established { session_id, available_modes } => {
                if let Ok(mut sessions) = self.sessions.lock() {
                    let entry = sessions.entry(terminal).or_default();
                    entry.session_id = Some(session_id);
                    entry.available_modes = available_modes;
                }
                return;
            }
            ShimMessage::Failed { reason } => {
                tracing::warn!(terminal = %terminal, %reason, "agent adapter failed to start");
                return;
            }
        };

        if let Ok(mut sessions) = self.sessions.lock() {
            let entry = sessions.entry(terminal).or_default();
            for s in &batch {
                entry.activity = fold_activity(entry.activity, &s.event);
                entry.cursor = s.seq + 1;
            }
        }
        on_events(terminal, batch);
    }

    /// A client looked at this terminal, which is what ends `Done`.
    pub fn seen(&self, terminal: Uuid) {
        if let Ok(mut sessions) = self.sessions.lock() {
            if let Some(entry) = sessions.get_mut(&terminal) {
                entry.activity = activity::seen(entry.activity);
            }
        }
    }
}
```

Add to `Service` in `crates/daemon/src/service.rs`:

```rust
    /// Toggle a terminal between hosting a TUI and hosting an ACP agent.
    ///
    /// The pane is respawned rather than replaced, so the terminal keeps its
    /// id, its tag and its rectangle, and a chat opening in one tile of four
    /// does not rearrange the other three.
    pub async fn set_pane_mode(
        &self,
        id: Uuid,
        pane_mode: models::PaneMode,
        force: bool,
    ) -> Result<models::Terminal> {
        let term = self.store.get_terminal(id)?;
        let ws = self.store.get_workspace(term.workspace_id)?;

        // `ConfirmationRequired` rather than a new code: a turn in flight is
        // exactly the existing "tell the user what this destroys and ask", and
        // `force` is the confirmation coming back.
        agent_supervisor::guard_toggle(self.agents.activity(id), force)
            .map_err(|_| DomainError::ConfirmationRequired)?;

        let snapshot = self.inventory.refresh().await;
        let pane = snapshot
            .claimants(id)
            .into_iter()
            .find(|p| p.proves_life())
            .ok_or(DomainError::NotFound)?;

        // A session id already declared at launch is reused; a hand-started
        // agent is looked up, and an ambiguous lookup refuses rather than
        // attaching a chat to the wrong conversation.
        let session_id = match term.agent_session_id.clone() {
            Some(existing) => Some(existing),
            None => {
                let home = directories::UserDirs::new()
                    .map(|d| d.home_dir().to_path_buf())
                    .ok_or(DomainError::NotFound)?;
                session_discovery::discover_claude_session(
                    &home,
                    std::path::Path::new(&ws.worktree_path),
                    std::time::SystemTime::UNIX_EPOCH,
                )
                .ok()
            }
        };

        let command = match pane_mode {
            models::PaneMode::Terminal => {
                let sid = session_id.clone().unwrap_or_default();
                let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
                if Uuid::parse_str(&sid).is_ok() {
                    format!("{shell} -ilc 'claude --resume {sid}'")
                } else {
                    preset_command(&term.command_preset, None)
                }
            }
            models::PaneMode::Agent => {
                let binary = std::env::current_exe()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "farcooler".to_string());
                let socket =
                    agent_supervisor::socket_path(&self.runtime_dir, id).display().to_string();
                let session = session_id
                    .as_deref()
                    .map(|s| format!(" --session {s}"))
                    .unwrap_or_default();
                format!(
                    "{binary} agent-host --terminal {id} --socket {socket} --worktree {} {session}",
                    ws.worktree_path
                )
            }
        };

        self.tmux.respawn_pane(&pane.pane_id, &ws.worktree_path, &command).await?;
        self.store.set_pane_mode(id, term.resource_version, pane_mode, session_id)
    }
```

Add `agents: agent_supervisor::AgentSupervisor` and `runtime_dir: PathBuf` fields to `Service`, initialized in `open_in`, and add `use crate::{agent_supervisor, session_discovery};` at the top of `service.rs`.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-daemon agent_supervisor`
Expected: PASS, 4 tests

Then: `cargo test --workspace`

- [ ] **Step 5: Commit**

```bash
git add crates/daemon/src/agent_supervisor.rs crates/daemon/src/service.rs crates/daemon/src/lib.rs
git commit -m "feat(daemon): supervise agent sessions, and refuse a toggle mid-turn"
```

---

### Task 16: Protocol surface for clients

**Files:**
- Modify: `proto/farcooler.proto`
- Modify: `crates/daemon/src/rpc.rs`
- Modify: `crates/daemon/src/wire.rs`

**Interfaces:**
- Produces: `Terminal.pane_mode`, `Terminal.agent_session_id`, requests `SetPaneMode`, `AgentSubscribe`, `AgentPrompt`, `AgentAnswer`, `AgentSetMode`, `AgentCancel`, `WorktreeFileSearch`, result `AgentEventBatch`, and `Event.agent_events`.

- [ ] **Step 1: Write the failing test**

Add to `crates/daemon/tests/rpc_over_socket.rs`:

```rust
#[tokio::test]
async fn a_terminal_reports_its_pane_mode_to_a_client() {
    // Clients render pane mode; they must never infer it from a command line.
    let harness = Harness::start().await;
    let terminal = harness.create_terminal("claude").await;
    assert_eq!(terminal.pane_mode, PaneMode::Terminal as i32);
}

#[tokio::test]
async fn an_agent_subscribe_from_a_cursor_is_accepted() {
    let harness = Harness::start().await;
    let terminal = harness.create_terminal("claude").await;
    let response = harness
        .request(request::Payload::AgentSubscribe(AgentSubscribe {
            terminal_id: terminal.id.clone(),
            from_seq: 0,
        }))
        .await;
    assert!(response.is_ok(), "subscribe must be accepted even before a session exists");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p farcooler-daemon --test rpc_over_socket`
Expected: FAIL — `no field pane_mode on type Terminal`

- [ ] **Step 3: Extend the proto and wire it up**

In `proto/farcooler.proto`, add to `message Terminal` after field 17:

```proto
  // What this terminal's pane is HOSTING: a TUI, or a headless ACP agent.
  //
  // Distinct from the VT `mode` a terminal reports and from the ACP
  // `agent_mode` below. Three unrelated things in one pane would otherwise all
  // be called mode.
  PaneMode pane_mode = 18;

  // The ACP session this terminal is for. Durable intent: it outlives every
  // pane that ever hosts it, which is what makes toggling pane mode land on
  // the same conversation.
  optional string agent_session_id = 19;

  // The ACP mode the agent is in, and the ones it offers. Empty in TERMINAL
  // pane mode.
  optional string agent_mode = 20;
  repeated string available_agent_modes = 21;
}

enum PaneMode {
  PANE_MODE_UNSPECIFIED = 0;
  // A TUI. The default, and the mode that needs no adapter installed.
  PANE_MODE_TERMINAL = 1;
  PANE_MODE_AGENT = 2;
```

Add the new messages near the terminal channel section:

```proto
// ---------------------------------------------------------------------------
// Agent channel
//
// A sequence number counts EVENTS, not bytes: unlike a terminal stream these
// have record boundaries, so a cursor can name one.
// ---------------------------------------------------------------------------

message SetPaneMode {
  bytes terminal_id = 1;
  PaneMode pane_mode = 2;
  // Switching to TERMINAL mid-turn discards work `claude --resume` cannot
  // reattach to, so it is refused unless the user has been told and agreed.
  bool force = 3;
}

message AgentSubscribe {
  bytes terminal_id = 1;
  uint64 from_seq = 2;
}

message AgentPromptBlock {
  oneof content {
    string text = 1;
    // A worktree-relative path from an @-mention.
    string file_mention = 2;
    ImageBlock image = 3;
  }
}

message ImageBlock {
  string mime_type = 1;
  bytes data = 2;
}

message AgentPrompt {
  bytes terminal_id = 1;
  repeated AgentPromptBlock blocks = 2;
}

message AgentAnswer {
  bytes terminal_id = 1;
  string request_id = 2;
  string option_id = 3;
}

message AgentSetMode {
  bytes terminal_id = 1;
  string agent_mode = 2;
}

message AgentCancel { bytes terminal_id = 1; }

message WorktreeFileSearch {
  bytes workspace_id = 1;
  string query = 2;
  uint32 limit = 3;
}

message WorktreeFileList { repeated string paths = 1; }

// Why part of a transcript is missing. Rendered as a visible break, never
// silently omitted — the same contract as a lost terminal.
enum AgentGapReason {
  AGENT_GAP_REASON_UNSPECIFIED = 0;
  AGENT_GAP_REASON_RING_TRIMMED = 1;
  AGENT_GAP_REASON_LOAD_UNSUPPORTED = 2;
  AGENT_GAP_REASON_UNPARSED = 3;
}

message AgentEventBatch {
  bytes terminal_id = 1;
  repeated AgentEventFrame events = 2;
}

message AgentEventFrame {
  uint64 seq = 1;
  // The normalized event, as JSON. The shape is `farcooler_agent::event::
  // AgentEvent`, which is the single definition both apps decode.
  string payload_json = 2;
}
```

Add to `Request.payload`:

```proto
    SetPaneMode set_pane_mode = 33;
    AgentSubscribe agent_subscribe = 34;
    AgentPrompt agent_prompt = 35;
    AgentAnswer agent_answer = 36;
    AgentSetMode agent_set_mode = 37;
    AgentCancel agent_cancel = 38;
    WorktreeFileSearch worktree_file_search = 39;
```

Add to `Result.value`:

```proto
    AgentEventBatch agent_event_batch = 17;
    WorktreeFileList worktree_file_list = 18;
```

Add to `Event.payload`:

```proto
    AgentEventBatch agent_events = 17;
```

In `crates/daemon/src/wire.rs`, add the mapping and populate the new `Terminal` fields:

```rust
/// Durable intent to the wire. Unspecified is never sent: a client that cannot
/// tell which mode a pane is in would not know which surface to draw.
pub fn pane_mode(mode: models::PaneMode) -> i32 {
    match mode {
        models::PaneMode::Terminal => v1::PaneMode::Terminal as i32,
        models::PaneMode::Agent => v1::PaneMode::Agent as i32,
    }
}
```

and, where the `v1::Terminal` is built:

```rust
        pane_mode: pane_mode(record.pane_mode),
        agent_session_id: record.agent_session_id.clone(),
        agent_mode: agents.agent_mode(record.id),
        available_agent_modes: agents.available_modes(record.id),
```

In `crates/daemon/src/rpc.rs`, add one arm per payload to the existing `dispatch` match:

```rust
            Payload::SetPaneMode(req) => {
                let id = ids::terminal(&req.terminal_id)?;
                let mode = match v1::PaneMode::try_from(req.pane_mode) {
                    Ok(v1::PaneMode::Agent) => models::PaneMode::Agent,
                    Ok(v1::PaneMode::Terminal) => models::PaneMode::Terminal,
                    // A client that sends UNSPECIFIED is asking for a mode that
                    // does not exist, not for a default.
                    _ => return Err(DomainError::InvalidArgument { what: "pane_mode" }),
                };
                let term = self.service.set_pane_mode(id, mode, req.force).await?;
                Ok(result::Value::Terminal(wire::terminal(&term, self.service.agents())))
            }
            Payload::AgentSubscribe(req) => {
                let id = ids::terminal(&req.terminal_id)?;
                // Subscribing before a session exists is legal and returns an
                // empty batch: a client attaches to a pane, not to a session.
                let events = self.service.agents().replay(id, req.from_seq);
                Ok(result::Value::AgentEventBatch(wire::agent_batch(id, events)))
            }
            Payload::AgentPrompt(req) => {
                let id = ids::terminal(&req.terminal_id)?;
                self.service.agents().send(id, DaemonMessage::Prompt { text: wire::prompt_text(&req.blocks) });
                Ok(result::Value::Empty(v1::Empty {}))
            }
            Payload::AgentAnswer(req) => {
                let id = ids::terminal(&req.terminal_id)?;
                self.service.agents().send(
                    id,
                    DaemonMessage::Answer { request_id: req.request_id, option_id: req.option_id },
                );
                self.service.agents().seen(id);
                Ok(result::Value::Empty(v1::Empty {}))
            }
            Payload::AgentSetMode(req) => {
                let id = ids::terminal(&req.terminal_id)?;
                self.service.agents().send(id, DaemonMessage::SetMode { agent_mode: req.agent_mode });
                Ok(result::Value::Empty(v1::Empty {}))
            }
            Payload::AgentCancel(req) => {
                let id = ids::terminal(&req.terminal_id)?;
                self.service.agents().send(id, DaemonMessage::Cancel);
                Ok(result::Value::Empty(v1::Empty {}))
            }
            Payload::WorktreeFileSearch(req) => {
                let id = ids::workspace(&req.workspace_id)?;
                let paths = self.service.search_worktree_files(id, &req.query, req.limit).await?;
                Ok(result::Value::WorktreeFileList(v1::WorktreeFileList { paths }))
            }
```

Add the two supporting `wire.rs` helpers — `agent_batch` serializing each `Sequenced` with `serde_json::to_string`, and `prompt_text` joining blocks (a `file_mention` renders as `@path`, an `image` is carried through to a later slice and contributes no text).

Add to `AgentSupervisor` in `crates/daemon/src/agent_supervisor.rs`:

```rust
    pub fn agent_mode(&self, terminal: Uuid) -> Option<String> {
        self.sessions.lock().ok().and_then(|s| s.get(&terminal).and_then(|st| st.agent_mode.clone()))
    }

    pub fn available_modes(&self, terminal: Uuid) -> Vec<String> {
        self.sessions
            .lock()
            .ok()
            .map(|s| s.get(&terminal).map(|st| st.available_modes.clone()).unwrap_or_default())
            .unwrap_or_default()
    }

    /// Events at and after `from_seq`, from the daemon's recent window.
    ///
    /// Empty for a terminal with no session — attaching to a pane that is not
    /// in agent mode is not an error, it just has nothing to show yet.
    pub fn replay(&self, terminal: Uuid, from_seq: Seq) -> Vec<farcooler_agent::event::Sequenced> {
        self.recent
            .lock()
            .ok()
            .and_then(|r| r.get(&terminal).cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|e| e.seq >= from_seq)
            .collect()
    }
```

with `agent_mode: Option<String>` added to `SessionState` (set from `AgentEvent::ModeSet` and `SessionStarted` in `apply`), and `recent: Arc<Mutex<HashMap<Uuid, Vec<Sequenced>>>>` added to `AgentSupervisor`, appended in `apply` and truncated to the most recent 512 entries per terminal. The shim's ring remains the authority; this is only the fast-attach window named in the spec.

Add to `Service` in `crates/daemon/src/service.rs`:

```rust
    /// Files in a workspace's worktree, for the `@`-mention picker.
    ///
    /// Substring match on the worktree-relative path, capped. Deliberately not
    /// a git call: an untracked file the agent just created is exactly the one
    /// a user wants to mention next.
    pub async fn search_worktree_files(
        &self,
        workspace_id: Uuid,
        query: &str,
        limit: u32,
    ) -> Result<Vec<String>> {
        let ws = self.store.get_workspace(workspace_id)?;
        let root = std::path::PathBuf::from(&ws.worktree_path);
        let needle = query.to_lowercase();
        let cap = if limit == 0 { 50 } else { limit.min(500) } as usize;

        let mut out = Vec::new();
        let mut stack = vec![root.clone()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                // `.git` is enormous and never mentionable.
                if name == ".git" || name == "target" || name == "node_modules" {
                    continue;
                }
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let Ok(relative) = path.strip_prefix(&root) else { continue };
                let relative = relative.display().to_string();
                if needle.is_empty() || relative.to_lowercase().contains(&needle) {
                    out.push(relative);
                    if out.len() >= cap {
                        return Ok(out);
                    }
                }
            }
        }
        Ok(out)
    }
```

Add to `crates/daemon/src/service.rs`'s test module:

```rust
    #[tokio::test]
    async fn worktree_search_finds_a_file_that_git_has_never_seen() {
        // The @-mention case that matters: the file the agent just created.
        let service = temp_service().await;
        let ws = seed_workspace(&service).await;
        std::fs::write(
            std::path::Path::new(&ws.worktree_path).join("brand_new.rs"),
            "fn main() {}",
        )
        .unwrap();
        let hits = service.search_worktree_files(ws.id, "brand", 10).await.unwrap();
        assert_eq!(hits, vec!["brand_new.rs".to_string()]);
    }

    #[tokio::test]
    async fn worktree_search_never_offers_the_git_directory() {
        let service = temp_service().await;
        let ws = seed_workspace(&service).await;
        let hits = service.search_worktree_files(ws.id, "", 500).await.unwrap();
        assert!(!hits.iter().any(|p| p.starts_with(".git/")), "{hits:?}");
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p farcooler-daemon --test rpc_over_socket`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add proto/ crates/daemon/src/rpc.rs crates/daemon/src/wire.rs
git commit -m "feat(protocol): the agent channel, with a gap reason clients must render"
```

---

### Task 17: The same behavior on every transport

**Files:**
- Modify: `crates/daemon/tests/stdio_transport.rs`
- Create: `crates/agent/tests/properties.rs`

**Interfaces:**
- Consumes: everything above.

The design doc's rule: a behavior is not complete if it passes locally through a path the remote adapter cannot exercise. The phone reaches the daemon over SSH stdio, so the agent channel must be proven there and not only on the Unix socket.

- [ ] **Step 1: Write the failing tests**

Add to `crates/daemon/tests/stdio_transport.rs`, mirroring the assertions added to `rpc_over_socket.rs` in Task 16:

```rust
#[tokio::test]
async fn the_agent_channel_behaves_identically_over_stdio() {
    // The phone IS this path. A feature that only works on the Unix socket is
    // a feature the product does not have.
    let harness = StdioHarness::start().await;
    let terminal = harness.create_terminal("claude").await;
    assert_eq!(terminal.pane_mode, PaneMode::Terminal as i32);

    let response = harness
        .request(request::Payload::AgentSubscribe(AgentSubscribe {
            terminal_id: terminal.id.clone(),
            from_seq: 0,
        }))
        .await;
    assert!(response.is_ok());
}

#[tokio::test]
async fn a_pane_mode_toggle_is_refused_over_stdio_the_same_way() {
    // Authorization and precondition outcomes must not differ by transport.
    let harness = StdioHarness::start().await;
    let terminal = harness.create_terminal("claude").await;
    let response = harness
        .request(request::Payload::SetPaneMode(SetPaneMode {
            terminal_id: vec![0u8; 16],
            pane_mode: PaneMode::Agent as i32,
            force: false,
        }))
        .await;
    assert!(response.is_err(), "an unknown terminal must fail identically on both adapters");
    let _ = terminal;
}
```

Create `crates/agent/tests/properties.rs`:

```rust
//! The three invariants the whole design rests on.

use farcooler_agent::event::{AgentEvent, AgentGapReason, Role};
use farcooler_agent::ring::{AgentReplay, AgentRing};

fn msg(i: usize) -> AgentEvent {
    AgentEvent::Message { role: Role::Agent, text: format!("m{i}") }
}

#[test]
fn sequence_numbers_are_monotonic_across_any_number_of_pushes() {
    let mut ring = AgentRing::with_capacity(16);
    let mut last = None;
    for i in 0..1000 {
        let seq = ring.push(msg(i));
        if let Some(prev) = last {
            assert!(seq > prev, "seq went backwards at {i}");
        }
        last = Some(seq);
    }
}

#[test]
fn replay_from_any_retained_cursor_reconstructs_an_identical_transcript() {
    let mut ring = AgentRing::with_capacity(64);
    for i in 0..50 {
        ring.push(msg(i));
    }
    let AgentReplay::At { events: all } = ring.since(0) else { panic!("nothing was trimmed") };

    for cursor in 0..50u64 {
        let AgentReplay::At { events } = ring.since(cursor) else { panic!("cursor {cursor}") };
        let expected: Vec<_> = all.iter().filter(|e| e.seq >= cursor).cloned().collect();
        assert_eq!(events, expected, "replay from {cursor} diverged");
    }
}

#[test]
fn dropped_history_is_always_a_gap_and_never_a_shorter_transcript() {
    // The contract that makes a derived transcript permissible in a product
    // that refuses to guess. A reader must never receive a short list that
    // looks complete.
    let mut ring = AgentRing::with_capacity(8);
    for i in 0..100 {
        ring.push(msg(i));
    }
    match ring.since(0) {
        AgentReplay::Gap { resumed_at, dropped, events } => {
            assert_eq!(dropped, resumed_at);
            assert_eq!(events.len(), 8);
        }
        AgentReplay::At { .. } => panic!("trimmed history reported as complete"),
    }
}

#[test]
fn an_unparsed_update_is_representable_end_to_end() {
    let mut ring = AgentRing::new();
    ring.push(AgentEvent::Gap { reason: AgentGapReason::Unparsed });
    let AgentReplay::At { events } = ring.since(0) else { panic!("retained") };
    assert!(matches!(events[0].event, AgentEvent::Gap { .. }));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p farcooler-daemon --test stdio_transport && cargo test -p farcooler-agent --test properties`
Expected: FAIL — the stdio harness does not yet know the new payload variants

- [ ] **Step 3: Make them pass**

Extend `StdioHarness` in `crates/daemon/tests/stdio_transport.rs` with the same `create_terminal` and `request` helpers `rpc_over_socket.rs` uses, so both files exercise one dispatch path. No production code should need changing: if it does, the Unix-socket path grew a shortcut the stdio path cannot take, and that is the bug this task exists to find — fix it in `crates/daemon/src/rpc.rs` rather than in the harness.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS, whole workspace

- [ ] **Step 5: Commit**

```bash
git add crates/daemon/tests/ crates/agent/tests/properties.rs
git commit -m "test: the agent channel is the same behavior on every transport"
```

---

## What this plan deliberately does not do

- No macOS or iOS UI. `AgentSurface`, the shared Swift package, slash commands, `@`-mentions, images and the mode switcher are the second plan, written against the contract settled here.
- No Codex or Cursor adapter. The normalizer is where they will attach; adding them now would mean designing per-capability degradation against three unknowns instead of one known agent.
- No syntax highlighting in diffs, no per-hunk accept/reject, no checkpoint/rewind — cut in the spec.
- `crates/core/src/activity.rs` is not modified. Its screen classifier still owns terminal pane mode, and `advance`, `seen` and `wants_attention` are reused by the supervisor unchanged.
