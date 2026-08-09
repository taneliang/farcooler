# Native SDK Backend Seam Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cut a `Backend` seam under chat mode so `claude` and `codex` can be
driven through their own protocols, while `cursor`, `opencode`, and every
config-added adapter keep speaking ACP unchanged.

**Architecture:** A base crate holds the vendor-neutral vocabulary and path
confinement. Each protocol becomes its own crate depending only on that base.
`farcooler-agent` keeps the ring, the daemon link, the prompt queue, and a
`Backend` enum that dispatches to one of three implementations.

**Tech Stack:** Rust 2024 edition, rust-version 1.85, tokio, serde/serde_json,
thiserror. No new workspace dependencies.

## Global Constraints

- **US English throughout.** Never "behaviour", "colour", "normalise" — in code,
  comments, or user-facing copy.
- **Never run `cargo fmt`.** The Rust tree is hand-formatted and CI skips
  `fmt --check` deliberately.
- **`cargo` is not on PATH.** Prefix every command with
  `PATH="$HOME/.cargo/bin:$PATH"`.
- **No new workspace dependencies.** `crates/agent` has six today. Dispatch is an
  enum precisely so `async-trait` is not needed.
- **Zero proto changes and zero app changes.** If a task needs to touch
  `proto/farcooler.proto`, `apps/`, or any Swift/Kotlin file, the task is wrong.
- **Every moved file moves verbatim.** Adjust `use` paths only. A move that also
  edits logic makes the diff unreviewable and is a plan violation.
- **`cargo test --workspace` takes several minutes** and starts real `npx`
  agents. `an_exited_command_is_observed_as_dead_not_silently_gone` in
  `crates/tmux/tests/live_tmux.rs` is a known flake under parallel load; it
  passes in isolation and is not caused by this work.

## Deviations from the approved spec

Both are deliberate, and both are narrower than what the spec authorized.

1. **The base crate is `farcooler-agent-core`, not `farcooler-agent-event`.**
   `fs_guard::confine` has to live in it, because both the ACP and Codex
   backends answer `fs/*` requests and need confinement. A crate named "event"
   that contains path confinement is misnamed.
2. **Step 5's codegen is scoped to Codex.** `scripts/regen-backend-types.sh`
   pins both vendor artifacts and generates Rust for Codex's JSON Schema. Claude
   has no schema — only `sdk.d.ts` — and no off-the-shelf TypeScript-to-Rust
   generator exists, so writing one is its own project. The Claude handshake
   needs four structs, which Task 10 hand-writes against the pinned `.d.ts`.
   Full `.d.ts` codegen belongs to spec #2, where transcript types are consumed.

---

## File Structure

**Created:**
- `crates/agent-core/Cargo.toml`, `src/lib.rs`, `src/event.rs`, `src/fs_guard.rs`,
  `src/backend.rs` — vocabulary, confinement, `Capabilities`, `BackendError`.
- `crates/acp/Cargo.toml`, `src/lib.rs`, `src/conn.rs`, `src/wire.rs`,
  `src/normalize.rs`, `src/backend.rs` — today's ACP, plus its `AgentBackend` impl.
- `crates/codex/Cargo.toml`, `src/lib.rs`, `src/handshake.rs`, `src/generated.rs`
- `crates/claude/Cargo.toml`, `src/lib.rs`, `src/handshake.rs`
- `crates/agent/src/chat.rs` — the neutral session: prompt queue and turn state.
- `crates/agent/src/dispatch.rs` — the `Backend` enum and `Backend::Fake`.
- `scripts/regen-backend-types.sh`
- `vendor/codex-app-server-<version>.schema.json`, `vendor/claude-sdk-<version>.d.ts`

**Modified:**
- `crates/agent/src/lib.rs` — re-export `event`/`fs_guard`/`acp` so no downstream
  path changes.
- `crates/agent/src/session.rs` — shrinks to ACP-only startup, then moves.
- `crates/core/src/activity.rs` — `Backend` field on `AdapterSpec`; `handshake` moves out.
- `crates/core/src/config.rs` — parse `backend` in `[adapters.*]`.
- `crates/cli/src/agent_host.rs` — select a backend by preset.
- `Cargo.toml` — four new workspace members.

---

### Task 1: Extract `farcooler-agent-core`

**Files:**
- Create: `crates/agent-core/Cargo.toml`, `crates/agent-core/src/lib.rs`
- Move: `crates/agent/src/event.rs` → `crates/agent-core/src/event.rs`
- Move: `crates/agent/src/fs_guard.rs` → `crates/agent-core/src/fs_guard.rs`
- Modify: `crates/agent/src/lib.rs`, `Cargo.toml` (workspace members + deps)

**Interfaces:**
- Consumes: nothing.
- Produces: crate `farcooler_agent_core` exporting modules `event` and `fs_guard`.
  `farcooler_agent::event::*` and `farcooler_agent::fs_guard::*` keep resolving
  via re-export, so no other crate changes.

- [ ] **Step 1: Confirm the baseline passes for the crates being touched**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent`
Expected: PASS. Record the test count; Task 1 must not change it.

- [ ] **Step 2: Create the crate manifest**

`crates/agent-core/Cargo.toml`:

```toml
[package]
name = "farcooler-agent-core"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
serde.workspace = true
```

`fs_guard` uses only `std`, and `event` only `serde`. Nothing else belongs here.

- [ ] **Step 3: Move the two files verbatim**

```bash
git mv crates/agent/src/event.rs crates/agent-core/src/event.rs
git mv crates/agent/src/fs_guard.rs crates/agent-core/src/fs_guard.rs
```

Do not edit their contents. `event.rs` has no `crate::` references; check
`fs_guard.rs` for any and fix only those.

- [ ] **Step 4: Write the crate root**

`crates/agent-core/src/lib.rs`:

```rust
//! The vocabulary every backend produces, and the confinement they all need.
//!
//! A backend crate depends on this and on nothing else in the workspace. That
//! is what stops one protocol's types leaking into another's, and it is why
//! this crate has exactly one dependency.

pub mod event;
pub mod fs_guard;
```

- [ ] **Step 5: Register the member and the dependency**

In the workspace `Cargo.toml`, add `"crates/agent-core"` to `members` (before
`"crates/agent"`), and under `[workspace.dependencies]`:

```toml
farcooler-agent-core = { path = "crates/agent-core" }
```

In `crates/agent/Cargo.toml`, add to `[dependencies]`:

```toml
farcooler-agent-core.workspace = true
```

- [ ] **Step 6: Re-export from `farcooler-agent` so nothing downstream moves**

In `crates/agent/src/lib.rs`, replace the `pub mod event;` and `pub mod fs_guard;`
lines with:

```rust
/// Re-exported so `farcooler_agent::event::AgentEvent` keeps resolving.
///
/// The vocabulary moved to `farcooler-agent-core` so backend crates can depend
/// on it without depending on the orchestration above them. Every consumer —
/// the daemon, the CLI, the client — still spells it the way it always did,
/// because a crate split is not a reason to churn call sites.
pub use farcooler_agent_core::{event, fs_guard};
```

- [ ] **Step 7: Build and test**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent-core -p farcooler-agent`
Expected: PASS, with `event.rs`'s two tests now reported under
`farcooler-agent-core` and the same total as Step 1.

- [ ] **Step 8: Verify no other crate needed changing**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo build --workspace`
Expected: PASS with no edits to `crates/daemon`, `crates/cli`, or `crates/client`.
If any failed, the re-export in Step 6 is wrong — fix it there, not at the call site.

- [ ] **Step 9: Commit**

```bash
git add -A
git commit -m "refactor: the agent vocabulary becomes its own crate

A backend crate needs AgentEvent and fs_guard::confine and must not
need the orchestration above them. farcooler-agent re-exports both, so
every existing path still resolves and no consumer changed."
```

---

### Task 2: `Capabilities`, `BackendError`, and the `AgentBackend` trait

**Files:**
- Create: `crates/agent-core/src/backend.rs`
- Modify: `crates/agent-core/src/lib.rs`

**Interfaces:**
- Consumes: `event::{AgentEvent, PromptImage}` from Task 1.
- Produces: `BackendKind`, `Capabilities`, `BackendError`, `trait AgentBackend`.

- [ ] **Step 1: Write the failing test**

Append to `crates/agent-core/src/backend.rs` (created in this step, tests first):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_incompatible_backend_names_both_versions() {
        // The whole point of this variant: a user reading it has to be able to
        // tell which side is behind without running anything else.
        let e = BackendError::Incompatible {
            found: "0.152.0".into(),
            expected: "0.146.0".into(),
        };
        let text = e.to_string();
        assert!(text.contains("0.152.0"), "{text}");
        assert!(text.contains("0.146.0"), "{text}");
    }

    #[test]
    fn a_refusal_carries_the_agents_own_words() {
        // Anything else substitutes our description for the agent's, and the
        // agent usually said something more useful than we can.
        let e = BackendError::Refused("no rollout found for thread id".into());
        assert!(e.to_string().contains("no rollout found for thread id"));
    }

    #[test]
    fn acp_does_not_claim_native_steering() {
        // ACP has no way to inject into a running turn. Claiming otherwise
        // makes the composer tell the user a queued prompt was delivered.
        assert!(!Capabilities::acp().native_steer);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent-core backend`
Expected: FAIL — `cannot find type BackendError in this scope`.

- [ ] **Step 3: Write the implementation above the test module**

Prepend to `crates/agent-core/src/backend.rs`:

```rust
//! What every backend has to be able to do, and how it fails.

use crate::event::{AgentEvent, PromptImage};

/// Which protocol a session is running on. For messages and logs only —
/// never a reason to branch on behavior. Behavior is what `Capabilities` is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Acp,
    Claude,
    Codex,
}

impl BackendKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BackendKind::Acp => "acp",
            BackendKind::Claude => "claude",
            BackendKind::Codex => "codex",
        }
    }
}

/// What a backend can do, as distinct from what it currently offers.
///
/// Deliberately behavioral only. Modes, models, and config options are NOT
/// capabilities — they arrive dynamically on `SessionStarted`, and a client
/// renders whatever is in that list without knowing in advance what is in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    pub backend: BackendKind,
    /// The backend can inject a prompt into a turn already running.
    ///
    /// False means the neutral layer's queue emulates it by holding the prompt
    /// until `TurnEnded`. The distinction reaches the UI because a composer
    /// that says "sent" about a prompt still sitting in a queue is lying.
    pub native_steer: bool,
    /// The backend can rejoin a session it did not itself start.
    ///
    /// False makes the neutral layer emit `AgentGapReason::LoadUnsupported`
    /// without attempting a request it already knows will fail.
    pub replay: bool,
    /// The backend asks the client to perform file operations, so paths it
    /// reports must be confined before anything touches the disk.
    pub client_side_fs: bool,
}

impl Capabilities {
    /// ACP: no steering, confinement required, replay decided at `initialize`.
    pub fn acp() -> Self {
        Capabilities {
            backend: BackendKind::Acp,
            native_steer: false,
            replay: false,
            client_side_fs: true,
        }
    }
}

/// Why a backend could not do what was asked.
///
/// Four of these existed already under other names — `Status::AdapterMissing`,
/// `Status::AdapterSilent`, and two `AcpError` variants. `Incompatible` is new
/// and native-only: a generated protocol has a version, and the installed CLI
/// may not match it.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("could not start the agent")]
    Spawn,
    #[error("the agent started but never answered")]
    Silent,
    #[error("the agent closed its connection")]
    Closed,
    /// Carries the agent's own message, because the caller usually cannot say
    /// anything more useful than the agent already did.
    #[error("the agent refused: {0}")]
    Refused(String),
    /// The installed CLI speaks a protocol these generated types do not cover.
    #[error("this agent speaks protocol {found}, but this build was generated against {expected}")]
    Incompatible { found: String, expected: String },
}

/// One live agent conversation, whatever protocol carries it.
///
/// There is deliberately no `set_mode` and no `set_model`. `ConfigOption`
/// already argues the case: the agent advertises its selectors and the client
/// renders one control each "rather than the client knowing in advance that
/// 'mode' and 'model' exist". `mode` and `model` are well-known ids on
/// `set_config_option`, not methods.
///
/// Dispatched through an enum rather than as a trait object: async fn in traits
/// is not dyn-compatible, and the alternative is a new dependency and a boxed
/// future per call. The trait states the contract; `Backend` performs it.
pub trait AgentBackend: Send {
    fn capabilities(&self) -> Capabilities;

    /// Start a turn.
    fn prompt(
        &mut self,
        text: &str,
        images: &[PromptImage],
    ) -> impl std::future::Future<Output = Result<(), BackendError>> + Send;

    /// Inject into the turn already running.
    ///
    /// Only called when `capabilities().native_steer` is true. A backend that
    /// reports false never sees this, because the neutral layer's queue holds
    /// the prompt instead.
    fn steer(
        &mut self,
        text: &str,
        images: &[PromptImage],
    ) -> impl std::future::Future<Output = Result<(), BackendError>> + Send;

    fn answer(
        &mut self,
        request_id: &str,
        option_id: &str,
    ) -> impl std::future::Future<Output = Result<(), BackendError>> + Send;

    fn set_config_option(
        &mut self,
        id: &str,
        value: &str,
    ) -> impl std::future::Future<Output = Result<(), BackendError>> + Send;

    fn cancel(&mut self) -> impl std::future::Future<Output = Result<(), BackendError>> + Send;

    /// Block until the backend has something to say.
    ///
    /// Never returns an empty vector — a caller that has to distinguish "no
    /// events yet" from "an event with nothing in it" will get it wrong, and
    /// an empty return in a loop is a spin.
    fn next_events(
        &mut self,
    ) -> impl std::future::Future<Output = Result<Vec<AgentEvent>, BackendError>> + Send;
}
```

Add `thiserror.workspace = true` to `crates/agent-core/Cargo.toml`, and
`pub mod backend;` to `crates/agent-core/src/lib.rs`.

- [ ] **Step 4: Run the tests**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent-core`
Expected: PASS, 5 tests (2 from `event.rs`, 3 new).

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(agent-core): the backend contract, and how a backend fails

Capabilities is behavioral only — modes and models arrive on
SessionStarted and are not capabilities. BackendError::Incompatible is
the one genuinely new failure: a generated protocol has a version and
the installed CLI may not match it."
```

---

### Task 3: Move ACP into `farcooler-acp`

**Files:**
- Create: `crates/acp/Cargo.toml`, `crates/acp/src/lib.rs`
- Move: `crates/agent/src/acp/{conn,wire,normalize}.rs` → `crates/acp/src/`
- Move: `crates/agent/tests/{normalize_fixtures,v064_fixture,subagent_fixtures}.rs`
  and `crates/agent/tests/fixtures/` → `crates/acp/tests/`
- Modify: `crates/agent/src/lib.rs`, `crates/agent/src/session.rs`, `Cargo.toml`

**Interfaces:**
- Consumes: `farcooler_agent_core::{event, fs_guard}`.
- Produces: crate `farcooler_acp` with `conn::{AcpConnection, AcpWriter, AcpError,
  Incoming, encode_frame}`, `wire::*`, `normalize::{relativize, update_to_events}`.
  `farcooler_agent::acp::*` keeps resolving via re-export.

- [ ] **Step 1: Create the manifest**

`crates/acp/Cargo.toml`:

```toml
[package]
name = "farcooler-acp"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
farcooler-agent-core.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tokio.workspace = true
```

- [ ] **Step 2: Move the sources and the fixtures verbatim**

```bash
mkdir -p crates/acp/src crates/acp/tests
git mv crates/agent/src/acp/conn.rs crates/acp/src/conn.rs
git mv crates/agent/src/acp/wire.rs crates/acp/src/wire.rs
git mv crates/agent/src/acp/normalize.rs crates/acp/src/normalize.rs
git rm crates/agent/src/acp/mod.rs
git mv crates/agent/tests/normalize_fixtures.rs crates/acp/tests/normalize_fixtures.rs
git mv crates/agent/tests/v064_fixture.rs crates/acp/tests/v064_fixture.rs
git mv crates/agent/tests/subagent_fixtures.rs crates/acp/tests/subagent_fixtures.rs
git mv crates/agent/tests/fixtures crates/acp/tests/fixtures
```

- [ ] **Step 3: Fix imports only**

In the three moved sources, rewrite `use crate::event::` to
`use farcooler_agent_core::event::` and `use crate::acp::` to `use crate::`.
In the three moved tests, rewrite `farcooler_agent::acp::` to `farcooler_acp::`
and `farcooler_agent::event::` to `farcooler_agent_core::event::`.

Change nothing else. If a test's expectations need editing, the move was wrong.

- [ ] **Step 4: Write the crate root**

`crates/acp/src/lib.rs`:

```rust
//! The Agent Client Protocol backend.
//!
//! One of three, and the only one that is not vendor-specific: an ACP adapter
//! is how any agent without a native backend reaches chat mode, including
//! every adapter a user adds in their own config file.

pub mod conn;
pub mod normalize;
pub mod wire;
```

- [ ] **Step 5: Register and re-export**

Add `"crates/acp"` to workspace `members` and
`farcooler-acp = { path = "crates/acp" }` to `[workspace.dependencies]`.
Add `farcooler-acp.workspace = true` to `crates/agent/Cargo.toml`.

In `crates/agent/src/lib.rs`, replace `pub mod acp;` with:

```rust
/// Re-exported so `farcooler_agent::acp::conn::AcpConnection` keeps resolving.
pub use farcooler_acp as acp;
```

- [ ] **Step 6: Point `session.rs` at the moved crate**

In `crates/agent/src/session.rs`, change the three ACP `use` lines to:

```rust
use farcooler_acp::conn::{AcpConnection, AcpError, AcpWriter, Incoming};
use farcooler_acp::normalize::{relativize, update_to_events};
use farcooler_acp::wire::Rpc;
```

and the two core ones to:

```rust
use farcooler_agent_core::event::{
    AgentChoice, AgentEvent, AgentGapReason, ConfigOption, Diff, EndReason, PermissionOption,
    PromptImage, QueuedPrompt, Role, ToolStatus,
};
use farcooler_agent_core::fs_guard::confine;
```

- [ ] **Step 7: Test**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-acp -p farcooler-agent`
Expected: PASS. The fixture tests must report the same counts as before the move.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor: ACP becomes a backend crate

Moved verbatim, fixtures included. farcooler-agent re-exports it as
acp, so every path a consumer spells still resolves."
```

---

### Task 4: `AcpBackend` — the trait implemented for ACP

**Files:**
- Create: `crates/acp/src/backend.rs`
- Modify: `crates/acp/src/lib.rs`, `crates/agent/src/session.rs`

**Interfaces:**
- Consumes: `RunningSession` from `crates/agent/src/session.rs`, `Capabilities`,
  `BackendError`, `AgentBackend`.
- Produces: `farcooler_acp::backend::AcpBackend`, implementing `AgentBackend`.
  Constructed by `AcpBackend::new(running: RunningSession, replay: bool)`.

**Note on where `RunningSession` lives.** It stays in `crates/agent` for this
task and is passed in. Task 5 moves the queue out of it; what remains is
ACP-only and moves here in Task 5's Step 6. Splitting the move from the
extraction keeps each diff reviewable.

- [ ] **Step 1: Write the failing test**

`crates/acp/src/backend.rs`, tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acp_reports_replay_only_when_the_agent_declared_it() {
        // `initialize` is the only place loadSession is advertised, and an
        // adapter that never claimed it must not be asked to replay: the
        // request fails and the failure reads as a broken session rather than
        // as an agent that simply cannot do this.
        assert!(!Capabilities { replay: false, ..Capabilities::acp() }.replay);
        assert!(Capabilities { replay: true, ..Capabilities::acp() }.replay);
    }

    #[test]
    fn acp_always_needs_path_confinement() {
        // The agent asks US to write files. Every path it names is untrusted
        // until confine() has agreed it is inside the worktree.
        assert!(Capabilities::acp().client_side_fs);
    }
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-acp backend`
Expected: FAIL — `file not found for module backend` or unresolved `Capabilities`.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/acp/src/backend.rs`:

```rust
//! `AgentBackend`, as ACP performs it.

use farcooler_agent_core::backend::{AgentBackend, BackendError, Capabilities};
use farcooler_agent_core::event::{AgentEvent, PromptImage};

use crate::conn::AcpError;

impl From<AcpError> for BackendError {
    fn from(e: AcpError) -> Self {
        match e {
            AcpError::Spawn => BackendError::Spawn,
            AcpError::Closed => BackendError::Closed,
            AcpError::Malformed => BackendError::Closed,
            AcpError::Refused(m) => BackendError::Refused(m),
        }
    }
}
```

The `AcpBackend` struct itself is written in Task 5 Step 6, once
`RunningSession` has had the queue removed and there is a single ACP-only type
to wrap. This task lands the error mapping and the capability tests, which are
what the rest of the seam compiles against.

- [ ] **Step 4: Add `pub mod backend;` to `crates/acp/src/lib.rs` and test**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-acp`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(acp): map AcpError onto BackendError

Malformed folds into Closed deliberately: a frame we cannot parse means
the conversation cannot continue, and two names for one outcome would
make callers branch on a distinction with no different action behind it."
```

---

### Task 5: The neutral `ChatSession` — queue and turn state

**Files:**
- Create: `crates/agent/src/chat.rs`
- Modify: `crates/agent/src/session.rs` (queue removed), `crates/acp/src/backend.rs`
- Modify: `crates/agent/src/lib.rs`

**Interfaces:**
- Consumes: `AgentBackend`, `Capabilities`, `BackendError`, `event::*`.
- Produces: `ChatSession<B: AgentBackend>` with `prompt`, `edit_queued`,
  `cancel_queued`, `steer_queued`, `next_events`, and `queue_len` (test-only view).

**The behavior being preserved.** `RunningSession` tracks a turn with
`pending_prompt: Option<u64>` — an ACP request id. `ChatSession` tracks the same
fact as a `bool` set when a prompt is sent and cleared on `TurnEnded`. Same
semantics, and it works for a backend whose turn ids look nothing like ACP's.
The subtlety worth keeping: steering must NOT clear turn state, because a
steering prompt joins the running turn rather than starting its own.

- [ ] **Step 1: Write the failing tests**

`crates/agent/src/chat.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use farcooler_agent_core::event::{EndReason, Role};

    /// A backend that records what it was asked to do and says nothing back.
    struct Fake {
        caps: Capabilities,
        sent: Vec<String>,
        steered: Vec<String>,
    }

    impl Fake {
        fn new(native_steer: bool) -> Self {
            Fake {
                caps: Capabilities { native_steer, ..Capabilities::acp() },
                sent: Vec::new(),
                steered: Vec::new(),
            }
        }
    }

    impl AgentBackend for Fake {
        fn capabilities(&self) -> Capabilities { self.caps }
        async fn prompt(&mut self, text: &str, _: &[PromptImage]) -> Result<(), BackendError> {
            self.sent.push(text.to_string());
            Ok(())
        }
        async fn steer(&mut self, text: &str, _: &[PromptImage]) -> Result<(), BackendError> {
            self.steered.push(text.to_string());
            Ok(())
        }
        async fn answer(&mut self, _: &str, _: &str) -> Result<(), BackendError> { Ok(()) }
        async fn set_config_option(&mut self, _: &str, _: &str) -> Result<(), BackendError> { Ok(()) }
        async fn cancel(&mut self) -> Result<(), BackendError> { Ok(()) }
        async fn next_events(&mut self) -> Result<Vec<AgentEvent>, BackendError> {
            Ok(vec![AgentEvent::TurnEnded { reason: EndReason::EndTurn }])
        }
    }

    #[tokio::test]
    async fn a_prompt_with_no_turn_running_goes_straight_out() {
        let mut s = ChatSession::new(Fake::new(false));
        let events = s.prompt("hello", Vec::new()).await.unwrap();
        assert!(events.is_empty(), "nothing to report: it was simply sent");
        assert_eq!(s.backend().sent, vec!["hello".to_string()]);
    }

    #[tokio::test]
    async fn a_prompt_during_a_turn_is_queued_not_sent() {
        // The bug this exists to prevent: a message typed while the agent is
        // working looked sent and might never have been.
        let mut s = ChatSession::new(Fake::new(false));
        s.prompt("first", Vec::new()).await.unwrap();
        let events = s.prompt("second", Vec::new()).await.unwrap();
        assert_eq!(s.backend().sent, vec!["first".to_string()]);
        assert!(matches!(events.as_slice(), [AgentEvent::PromptQueue { items }] if items.len() == 1));
    }

    #[tokio::test]
    async fn a_queued_prompt_is_sent_when_the_turn_ends() {
        let mut s = ChatSession::new(Fake::new(false));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("second", Vec::new()).await.unwrap();
        let events = s.next_events().await.unwrap();
        assert_eq!(s.backend().sent, vec!["first".to_string(), "second".to_string()]);
        assert!(events.iter().any(|e| matches!(e, AgentEvent::TurnEnded { .. })));
        assert!(events.iter().any(
            |e| matches!(e, AgentEvent::Message { role: Role::User, text, .. } if text == "second")
        ));
    }

    #[tokio::test]
    async fn steering_uses_the_backend_when_it_has_one() {
        let mut s = ChatSession::new(Fake::new(true));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("correction", Vec::new()).await.unwrap();
        let queued = s.queued_ids();
        s.steer_queued(&queued[0]).await.unwrap();
        assert_eq!(s.backend().steered, vec!["correction".to_string()]);
        assert_eq!(s.backend().sent, vec!["first".to_string()], "steering is not a new turn");
    }

    #[tokio::test]
    async fn steering_does_not_end_the_turn() {
        // pending state must survive: a steering prompt joins the running turn,
        // and clearing it would leave that turn with nothing to report its end.
        let mut s = ChatSession::new(Fake::new(true));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("correction", Vec::new()).await.unwrap();
        let queued = s.queued_ids();
        s.steer_queued(&queued[0]).await.unwrap();
        assert!(s.turn_in_flight(), "the original turn is still running");
    }

    #[tokio::test]
    async fn an_edited_prompt_keeps_its_place() {
        let mut s = ChatSession::new(Fake::new(false));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("typo", Vec::new()).await.unwrap();
        let queued = s.queued_ids();
        let events = s.edit_queued(&queued[0], "fixed");
        assert!(matches!(
            events.as_slice(),
            [AgentEvent::PromptQueue { items }] if items[0].text == "fixed"
        ));
    }

    #[tokio::test]
    async fn a_cancelled_prompt_is_never_sent() {
        let mut s = ChatSession::new(Fake::new(false));
        s.prompt("first", Vec::new()).await.unwrap();
        s.prompt("regret", Vec::new()).await.unwrap();
        let queued = s.queued_ids();
        s.cancel_queued(&queued[0]);
        s.next_events().await.unwrap();
        assert_eq!(s.backend().sent, vec!["first".to_string()]);
    }

    #[tokio::test]
    async fn an_unknown_queue_id_is_ignored_rather_than_an_error() {
        // The turn may have ended and sent it between the click and the message.
        let mut s = ChatSession::new(Fake::new(false));
        assert!(s.edit_queued("nope", "x").is_empty());
        assert!(s.cancel_queued("nope").is_empty());
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent chat`
Expected: FAIL — `cannot find type ChatSession`.

- [ ] **Step 3: Write `ChatSession`**

Prepend to `crates/agent/src/chat.rs`:

```rust
//! The half of a conversation no vendor owns: what is queued, and whose turn it is.
//!
//! Everything here was inside `RunningSession` and reachable only through a
//! live ACP subprocess. It is Far Cooler's own behavior rather than any
//! protocol's — a prompt held HERE is one that can still be shown, rewritten,
//! or taken back, which is the whole reason it is not handed to the agent to
//! sit on.

use std::collections::VecDeque;

use farcooler_agent_core::backend::{AgentBackend, BackendError, Capabilities};
use farcooler_agent_core::event::{AgentEvent, PromptImage, QueuedPrompt, Role};

pub struct ChatSession<B: AgentBackend> {
    backend: B,
    queue: VecDeque<QueuedPrompt>,
    /// Names the queued prompts. Monotonic and never reused, so an edit or a
    /// cancel cannot land on a different message than the one being looked at.
    next_queue_id: u64,
    /// Whether a turn is running.
    ///
    /// `RunningSession` tracked this as the ACP request id it was waiting on.
    /// A bool says the same thing and says it for a backend whose turn ids look
    /// nothing like ACP's.
    in_flight: bool,
}

impl<B: AgentBackend> ChatSession<B> {
    pub fn new(backend: B) -> Self {
        ChatSession { backend, queue: VecDeque::new(), next_queue_id: 0, in_flight: false }
    }

    pub fn capabilities(&self) -> Capabilities {
        self.backend.capabilities()
    }

    pub fn turn_in_flight(&self) -> bool {
        self.in_flight
    }

    /// Send a prompt, or hold it until the current turn ends.
    pub async fn prompt(
        &mut self,
        text: &str,
        images: Vec<PromptImage>,
    ) -> Result<Vec<AgentEvent>, BackendError> {
        // Anything already waiting goes first, even with no turn running: a
        // send that failed leaves its prompt at the head of the queue, and
        // without this the message being written now would overtake it and the
        // conversation would carry the user's words out of order.
        if !self.in_flight && !self.queue.is_empty() {
            let mut events = self.send_next_queued().await;
            self.push(text, images);
            events.push(self.queue_event());
            return Ok(events);
        }
        if self.in_flight {
            self.push(text, images);
            return Ok(vec![self.queue_event()]);
        }
        self.backend.prompt(text, &images).await?;
        self.in_flight = true;
        Ok(Vec::new())
    }

    /// Rewrite a prompt that has not been sent. Unknown ids are ignored: the
    /// turn may have ended and sent it between the click and the message.
    pub fn edit_queued(&mut self, id: &str, text: &str) -> Vec<AgentEvent> {
        let Some(entry) = self.queue.iter_mut().find(|q| q.id == id) else { return Vec::new() };
        entry.text = text.to_string();
        vec![self.queue_event()]
    }

    /// Take back a prompt that has not been sent.
    pub fn cancel_queued(&mut self, id: &str) -> Vec<AgentEvent> {
        let before = self.queue.len();
        self.queue.retain(|q| q.id != id);
        if self.queue.len() == before { return Vec::new() }
        vec![self.queue_event()]
    }

    /// Send a queued prompt NOW, into the turn already running.
    ///
    /// `in_flight` is deliberately left alone. A steering prompt joins the
    /// running turn rather than starting its own — clearing it would leave the
    /// original turn with nothing to report its end, and the pane would say
    /// Working forever.
    pub async fn steer_queued(&mut self, id: &str) -> Result<Vec<AgentEvent>, BackendError> {
        let Some(index) = self.queue.iter().position(|q| q.id == id) else {
            return Ok(Vec::new());
        };
        let queued = self.queue.remove(index).expect("index just found");

        let sent = if self.backend.capabilities().native_steer {
            self.backend.steer(&queued.text, &queued.images).await
        } else {
            self.backend.prompt(&queued.text, &queued.images).await
        };
        if let Err(e) = sent {
            self.queue.insert(index, queued);
            return Err(e);
        }

        Ok(vec![
            self.queue_event(),
            AgentEvent::Message { role: Role::User, text: queued.text, parent: None },
        ])
    }

    /// Wait for the backend, and drain the queue when a turn ends.
    pub async fn next_events(&mut self) -> Result<Vec<AgentEvent>, BackendError> {
        let mut events = self.backend.next_events().await?;
        if events.iter().any(|e| matches!(e, AgentEvent::TurnEnded { .. })) {
            self.in_flight = false;
            events.extend(self.send_next_queued().await);
        }
        Ok(events)
    }

    fn push(&mut self, text: &str, images: Vec<PromptImage>) {
        let id = self.next_queue_id;
        self.next_queue_id += 1;
        self.queue.push_back(QueuedPrompt { id: id.to_string(), text: text.to_string(), images });
    }

    fn queue_event(&self) -> AgentEvent {
        AgentEvent::PromptQueue { items: self.queue.iter().cloned().collect() }
    }

    /// The next queued prompt, sent now that the turn is over.
    ///
    /// Returns the user message too, because this is the moment it truly
    /// becomes part of the conversation — before this it was only waiting.
    async fn send_next_queued(&mut self) -> Vec<AgentEvent> {
        let Some(next) = self.queue.pop_front() else { return Vec::new() };
        let mut events = vec![self.queue_event()];
        match self.backend.prompt(&next.text, &next.images).await {
            Ok(()) => {
                self.in_flight = true;
                events.push(AgentEvent::Message {
                    role: Role::User,
                    text: next.text,
                    parent: None,
                });
                events
            }
            Err(_) => {
                // Put it back rather than lose it. `in_flight` stays false,
                // which is honest — no turn is running — and means the next
                // message retries this one first rather than leaving a stuck
                // entry nothing will ever drain.
                self.queue.push_front(next);
                vec![self.queue_event()]
            }
        }
    }
}

#[cfg(test)]
impl<B: AgentBackend> ChatSession<B> {
    fn backend(&self) -> &B {
        &self.backend
    }

    fn queued_ids(&self) -> Vec<String> {
        self.queue.iter().map(|q| q.id.clone()).collect()
    }
}
```

Add `pub mod chat;` to `crates/agent/src/lib.rs`.

- [ ] **Step 4: Run the tests**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent chat`
Expected: PASS, 8 tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(agent): the prompt queue, testable without a subprocess

Lifted out of RunningSession, where it could only be exercised through a
live npx adapter. Turn state becomes a bool rather than an ACP request
id, which is the same fact in a form a non-ACP backend can also report."
```

- [ ] **Step 6: Move ACP's half of `RunningSession` into `AcpBackend`**

Move `crates/agent/src/session.rs`'s `RunningSession` to
`crates/acp/src/backend.rs` as `AcpBackend`, deleting the queue fields
(`queue`, `next_queue_id`) and the four methods now in `ChatSession`
(`edit_queued`, `cancel_queued`, `steer_queued`, `send_next_queued`, `queue_event`).
Rename `send_prompt` to `prompt`, keep `pending_prompt`, and implement
`AgentBackend` over what remains. `steer` sends the same `session/prompt`
request but restores `pending_prompt` afterwards, which is what the old
`steer_queued` did inline.

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-acp -p farcooler-agent`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(acp): RunningSession becomes AcpBackend

What is left after the queue moved out is entirely ACP: a writer, a
frame receiver, and the request id whose response ends a turn."
```

---

### Task 6: The `Backend` enum and `backend` in the adapter table

**Files:**
- Create: `crates/agent/src/dispatch.rs`
- Modify: `crates/core/src/activity.rs`, `crates/core/src/config.rs`,
  `crates/cli/src/agent_host.rs`

**Interfaces:**
- Consumes: `AcpBackend`, `AgentBackend`, `ChatSession`.
- Produces: `enum Backend { Acp(AcpBackend) }` implementing `AgentBackend`;
  `activity::Backend` (config-level enum: `Acp`, `Native`) on `AdapterSpec`.

- [ ] **Step 1: Write the failing test in `crates/core/src/config.rs`'s test module**

```rust
#[test]
fn an_adapter_defaults_to_acp_when_it_says_nothing() {
    // Every preset shipping today speaks ACP. A file that predates this field
    // must keep working, and silence has to mean the behavior it had.
    let (path, _dir) = scratch_config("[adapters.codex]\nprogram = \"npx\"\n");
    let registry = load_into(Registry::built_in(), &path);
    let spec = registry.rules("codex").unwrap().adapter.as_ref().unwrap();
    assert_eq!(spec.backend, AdapterBackend::Acp);
}

#[test]
fn an_adapter_can_ask_for_the_native_backend() {
    let (path, _dir) = scratch_config(
        "[adapters.codex]\nbackend = \"native\"\nprogram = \"codex\"\n",
    );
    let registry = load_into(Registry::built_in(), &path);
    let spec = registry.rules("codex").unwrap().adapter.as_ref().unwrap();
    assert_eq!(spec.backend, AdapterBackend::Native);
}

#[test]
fn an_unknown_backend_name_falls_back_to_acp_rather_than_losing_the_adapter() {
    // Same contract as a malformed file: a typo in one field must not cost
    // this agent its chat mode entirely.
    let (path, _dir) = scratch_config(
        "[adapters.codex]\nbackend = \"nativ\"\nprogram = \"codex\"\n",
    );
    let registry = load_into(Registry::built_in(), &path);
    let spec = registry.rules("codex").unwrap().adapter.as_ref().unwrap();
    assert_eq!(spec.backend, AdapterBackend::Acp);
}
```

Match the existing test module's helpers for writing a scratch config; reuse
whatever `an_entry_with_no_program_is_refused_rather_than_launched` uses rather
than inventing a second helper.

- [ ] **Step 2: Run to verify failure**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-core backend`
Expected: FAIL — `no field backend on type AdapterSpec`.

- [ ] **Step 3: Add the field**

In `crates/core/src/activity.rs`, above `AdapterSpec`:

```rust
/// Which protocol Far Cooler speaks to this agent.
///
/// Defaults to `Acp` for every preset, including `claude` and `codex`: a
/// native backend that cannot yet hold a conversation must not become the
/// default just because it exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AdapterBackend {
    #[default]
    Acp,
    Native,
}

impl AdapterBackend {
    /// Anything unrecognized is `Acp`, for the same reason a malformed config
    /// file is ignored rather than fatal: one typo must not cost an agent its
    /// chat mode.
    pub fn parse(name: &str) -> Self {
        match name.trim() {
            "native" => AdapterBackend::Native,
            _ => AdapterBackend::Acp,
        }
    }
}
```

Add `pub backend: AdapterBackend,` to `AdapterSpec` and `#[serde(default)] pub
backend: Option<String>,` to `ConfigAdapter` in `config.rs`. In the merge that
builds an `AdapterSpec` from a `ConfigAdapter`, set
`backend: entry.backend.as_deref().map(AdapterBackend::parse).unwrap_or_default()`.
Every literal `AdapterSpec { .. }` in `activity.rs`'s built-in table needs
`backend: AdapterBackend::Acp`; the `npx` helper can supply it.

**On `args` under a native backend:** document on the field that when `backend`
is `Native`, `args` are appended AFTER the protocol flags the backend owns,
rather than being the complete argument vector. Nothing enforces this in Task 6
— the native backends are not reachable yet — but the doc comment is what stops
the meaning being invented differently in Task 9.

- [ ] **Step 4: Run the tests**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-core`
Expected: PASS.

- [ ] **Step 5: Write the dispatch enum**

`crates/agent/src/dispatch.rs`:

```rust
//! One backend, chosen at startup.
//!
//! An enum rather than `dyn AgentBackend`: async fn in traits is not
//! dyn-compatible, so a trait object costs a new dependency and a boxed future
//! per call. The set is closed by design — every config-added adapter is ACP —
//! so adding a variant is a code change either way.

use farcooler_acp::backend::AcpBackend;
use farcooler_agent_core::backend::{AgentBackend, BackendError, Capabilities};
use farcooler_agent_core::event::{AgentEvent, PromptImage};

pub enum Backend {
    Acp(AcpBackend),
}

impl AgentBackend for Backend {
    fn capabilities(&self) -> Capabilities {
        match self {
            Backend::Acp(b) => b.capabilities(),
        }
    }
    async fn prompt(&mut self, text: &str, images: &[PromptImage]) -> Result<(), BackendError> {
        match self {
            Backend::Acp(b) => b.prompt(text, images).await,
        }
    }
    async fn steer(&mut self, text: &str, images: &[PromptImage]) -> Result<(), BackendError> {
        match self {
            Backend::Acp(b) => b.steer(text, images).await,
        }
    }
    async fn answer(&mut self, request_id: &str, option_id: &str) -> Result<(), BackendError> {
        match self {
            Backend::Acp(b) => b.answer(request_id, option_id).await,
        }
    }
    async fn set_config_option(&mut self, id: &str, value: &str) -> Result<(), BackendError> {
        match self {
            Backend::Acp(b) => b.set_config_option(id, value).await,
        }
    }
    async fn cancel(&mut self) -> Result<(), BackendError> {
        match self {
            Backend::Acp(b) => b.cancel().await,
        }
    }
    async fn next_events(&mut self) -> Result<Vec<AgentEvent>, BackendError> {
        match self {
            Backend::Acp(b) => b.next_events().await,
        }
    }
}
```

Add `pub mod dispatch;` to `crates/agent/src/lib.rs`.

- [ ] **Step 6: Rewire `agent_host.rs`**

At `crates/cli/src/agent_host.rs:160`, keep the `AcpConnection::spawn` and the
90-second `AgentSession::start` timeout exactly as they are, then wrap the
result: `Backend::Acp(AcpBackend::new(agent.into_running(), can_load))` and hand
that to `ChatSession::new`. Replace the direct `RunningSession` calls in the
command loop with the `ChatSession` equivalents. `Status::AdapterMissing` and
`Status::AdapterSilent` are unchanged.

- [ ] **Step 7: Full workspace test**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test --workspace`
Expected: PASS, except the known `live_tmux` flake. Re-run that target alone to
confirm: `cargo test -p farcooler-tmux --test live_tmux`.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat: chat mode runs through a Backend, and the config can name one

backend defaults to acp for every preset including claude and codex —
a native backend that cannot hold a conversation yet must not become
the default merely by existing. Behavior is unchanged end to end."
```

---

### Task 7: Move the handshake, and the drift guard with it

**Files:**
- Modify: `crates/core/src/activity.rs` (handshake moves out)
- Create: `crates/acp/src/handshake.rs`
- Move: `crates/core/tests/adapters.rs` → `crates/agent/tests/backends.rs`

**Interfaces:**
- Produces: `farcooler_acp::handshake::handshake(spec, timeout) -> Result<Handshake, String>`,
  and `farcooler_agent::dispatch::handshake(spec, timeout)` dispatching by
  `spec.backend`.

- [ ] **Step 1: Move `handshake` and its unit tests verbatim**

Move `pub fn handshake` and `struct Handshake` from `crates/core/src/activity.rs`
to `crates/acp/src/handshake.rs`, together with the fake-based unit tests that
`activity.rs` documents as living beside it. `crate::programs::find` is in
`farcooler-core`, which `farcooler-acp` must not depend on — so take the
resolved program path as a parameter and let the caller resolve it.

- [ ] **Step 2: Add the dispatching entry point**

In `crates/agent/src/dispatch.rs`:

```rust
/// Prove an adapter can start and complete its own handshake.
///
/// One implementation, reached by both the Test button in the machine-settings
/// editor and by `every_built_in_backend_completes_a_handshake`. That property
/// is why this is not written twice.
pub fn handshake(
    spec: &farcooler_core::activity::AdapterSpec,
    timeout: std::time::Duration,
) -> Result<String, String> {
    match spec.backend {
        AdapterBackend::Acp => farcooler_acp::handshake::handshake(spec, timeout),
        AdapterBackend::Native => Err("no native backend is implemented yet".to_string()),
    }
}
```

`farcooler-agent` may depend on `farcooler-core`; the direction that must not
exist is `farcooler-core` → a backend crate.

- [ ] **Step 3: Move and rename the live test**

```bash
git mv crates/core/tests/adapters.rs crates/agent/tests/backends.rs
```

Rename `every_built_in_adapter_completes_an_acp_handshake` to
`every_built_in_backend_completes_a_handshake` and point it at
`farcooler_agent::dispatch::handshake`. Keep the module doc, keep
`no_built_in_adapter_is_deprecated_on_npm` unchanged, and keep the documented
rule that a missing program is a FAILURE and not a skip.

- [ ] **Step 4: Repoint the Test button**

Whatever in `crates/daemon` calls `activity::handshake` now calls
`farcooler_agent::dispatch::handshake`. The daemon already depends on
`farcooler-agent`.

- [ ] **Step 5: Test**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test --workspace`
Expected: PASS. The handshake test starts real `npx` adapters and is slow on a
cold cache.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "refactor: the handshake belongs to the backend that performs it

It was hoisted into core so the Test button and the live test could be
one implementation. They still are — dispatch::handshake is what both
reach — but core cannot dispatch to a backend crate without inverting
the dependency graph, so it moved rather than grew a match."
```

---

### Task 8: Pin the vendor artifacts

**Files:**
- Create: `scripts/regen-backend-types.sh`, `vendor/README.md`
- Create: `vendor/codex-app-server.schema.json`, `vendor/claude-sdk.d.ts`,
  `vendor/PINNED`

- [ ] **Step 1: Write the script**

`scripts/regen-backend-types.sh`:

```bash
#!/usr/bin/env bash
# Refresh the vendor protocol artifacts the native backends are generated
# against, and record the versions they came from.
#
# Run by a human, never by cargo. `cargo build` must not need the network and
# CI must not need npm to compile a Rust workspace, so the OUTPUT is committed
# and this script is how it gets there.
set -euo pipefail
cd "$(dirname "$0")/.."

command -v codex >/dev/null || { echo "codex is not installed"; exit 1; }
command -v npm   >/dev/null || { echo "npm is not installed"; exit 1; }

codex_version=$(codex --version | awk '{print $NF}')
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

codex app-server generate-json-schema --out "$tmp/codex" >/dev/null
cp "$tmp/codex/codex_app_server_protocol.v2.schemas.json" vendor/codex-app-server.schema.json

npm pack @anthropic-ai/claude-agent-sdk --pack-destination "$tmp" >/dev/null
tar xzf "$tmp"/anthropic-ai-claude-agent-sdk-*.tgz -C "$tmp"
cp "$tmp/package/sdk.d.ts" vendor/claude-sdk.d.ts
claude_sdk_version=$(node -p "require('$tmp/package/package.json').version")

cat > vendor/PINNED <<EOF
codex-cli $codex_version
@anthropic-ai/claude-agent-sdk $claude_sdk_version
EOF

echo "pinned:"; cat vendor/PINNED
```

`chmod +x scripts/regen-backend-types.sh`.

- [ ] **Step 2: Run it**

Run: `./scripts/regen-backend-types.sh`
Expected: `vendor/PINNED` names codex-cli 0.146.0 and claude-agent-sdk 0.3.226
(or newer, if the local installs have moved).

- [ ] **Step 3: Write `vendor/README.md`**

Explain that these files are inputs to the native backends, that they are
committed so the build needs no network, that `vendor/PINNED` is the version
`BackendError::Incompatible` compares against, and that Claude has no schema —
only a TypeScript declaration — which is why its types are hand-written for now.

- [ ] **Step 4: Commit**

```bash
git add -A
git commit -m "chore: pin the vendor protocol artifacts

Committed rather than fetched at build time: cargo build must not need
the network. vendor/PINNED is what a native handshake compares the
installed CLI against."
```

---

### Task 9: `farcooler-codex` — handshake only

**Files:**
- Create: `crates/codex/Cargo.toml`, `src/lib.rs`, `src/handshake.rs`
- Modify: `crates/agent/src/dispatch.rs`, `Cargo.toml`

- [ ] **Step 1: Write the failing test**

`crates/codex/src/handshake.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_initialize_request_omits_the_jsonrpc_member() {
        // codex app-server is JSON-RPC 2.0 in every respect except that it does
        // not put "jsonrpc" on the wire. Sending it anyway was not tested here
        // to be harmless, so this pins what was actually observed.
        let frame = initialize_request(1);
        assert!(frame.get("jsonrpc").is_none(), "{frame}");
        assert_eq!(frame["method"], "initialize");
        assert!(frame["params"]["clientInfo"]["name"].is_string());
    }

    #[test]
    fn a_version_outside_the_pin_is_incompatible_rather_than_a_guess() {
        assert!(matches!(
            check_version("0.152.0", "0.146.0"),
            Err(BackendError::Incompatible { .. })
        ));
        assert!(check_version("0.146.0", "0.146.0").is_ok());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-codex`
Expected: FAIL — the package does not exist yet.

- [ ] **Step 3: Create the crate and implement**

`crates/codex/Cargo.toml` mirrors `crates/acp/Cargo.toml`. In
`src/handshake.rs`, write `initialize_request(id: u64) -> serde_json::Value`
sending `initialize` with `clientInfo { name: "farcooler", version }`, and
`check_version(found: &str, expected: &str) -> Result<(), BackendError>`
returning `Incompatible` when the major.minor differ. Spawn with
`program` plus `["app-server"]` plus the spec's `args` appended after — the
order Task 6's doc comment specifies.

- [ ] **Step 4: Test, then wire into dispatch**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-codex`
Expected: PASS.

Then in `dispatch::handshake`, replace the `Native` arm's error with a call to
`farcooler_codex::handshake::handshake` when the preset is `codex`.

- [ ] **Step 5: Prove it against the real binary**

Add to `crates/agent/tests/backends.rs`:

```rust
#[test]
fn the_codex_native_backend_handshakes_against_the_installed_binary() {
    // A missing program is a FAILURE, not a skip, for the reason the ACP
    // handshake test gives: silently passing means the one test that can catch
    // this never runs where it matters.
    let spec = AdapterSpec {
        program: "codex".into(),
        args: Vec::new(),
        env: Default::default(),
        backend: AdapterBackend::Native,
    };
    let result = farcooler_agent::dispatch::handshake(&spec, std::time::Duration::from_secs(90));
    assert!(result.is_ok(), "codex app-server handshake failed: {result:?}");
}
```

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent --test backends`
Expected: PASS against the installed codex.

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(codex): a native handshake against codex app-server

Enough to prove the seam is cut in the right place, and no more: this
backend cannot hold a conversation yet, and codex still defaults to acp."
```

---

### Task 10: `farcooler-claude` — handshake only

**Files:**
- Create: `crates/claude/Cargo.toml`, `src/lib.rs`, `src/handshake.rs`
- Modify: `crates/agent/src/dispatch.rs`, `Cargo.toml`

- [ ] **Step 1: Write the failing test**

`crates/claude/src/handshake.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_launch_flags_are_the_ones_the_sdk_uses() {
        // Read off sdk.d.ts and the CLI's own --help rather than guessed:
        // stream-json in BOTH directions, and --verbose, which the CLI
        // requires alongside stream-json output.
        let args = launch_args(&[]);
        assert!(args.contains(&"--print".to_string()), "{args:?}");
        assert!(args.windows(2).any(|w| w == ["--input-format", "stream-json"]));
        assert!(args.windows(2).any(|w| w == ["--output-format", "stream-json"]));
        assert!(args.contains(&"--verbose".to_string()));
    }

    #[test]
    fn extra_args_are_appended_after_the_protocol_flags() {
        // The config field means "extra", not "instead of". A user pinning a
        // model must not be able to unset --output-format and break the wire.
        let args = launch_args(&["--model".into(), "opus".into()]);
        let tail = &args[args.len() - 2..];
        assert_eq!(tail, ["--model".to_string(), "opus".to_string()]);
    }

    #[test]
    fn a_control_response_is_recognized_by_its_type_and_id() {
        let frame = serde_json::json!({
            "type": "control_response",
            "response": { "subtype": "success", "request_id": "1" }
        });
        assert_eq!(control_response_id(&frame), Some("1".to_string()));
        assert_eq!(control_response_id(&serde_json::json!({"type": "assistant"})), None);
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-claude`
Expected: FAIL — the package does not exist yet.

- [ ] **Step 3: Create the crate and implement**

`launch_args(extra: &[String]) -> Vec<String>` returns
`["--print", "--input-format", "stream-json", "--output-format", "stream-json",
"--verbose"]` followed by `extra`. `control_response_id` reads
`frame["response"]["request_id"]` when `frame["type"] == "control_response"`.
The handshake spawns the resolved `claude`, sends the `control_request`
`initialize` shape declared by `SDKControlInitializeRequest` in
`vendor/claude-sdk.d.ts`, and waits for the matching `control_response`.

**Environment:** the daemon's `CLAUDECODE` variable must be scrubbed before
spawning, exactly as the ACP path already does — the CLI refuses to launch
nested inside another Claude Code session and neither answers nor exits, which
is the documented cause of `Status::AdapterSilent`.

- [ ] **Step 4: Test, then wire into dispatch**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-claude`
Expected: PASS.

Then add the `claude` preset to `dispatch::handshake`'s `Native` arm.

- [ ] **Step 5: Prove it against the real binary**

Add `the_claude_native_backend_handshakes_against_the_installed_binary` to
`crates/agent/tests/backends.rs`, mirroring Task 9 Step 5 with
`program: "claude"`.

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test -p farcooler-agent --test backends`
Expected: PASS against the installed claude.

- [ ] **Step 6: Full workspace test and commit**

Run: `PATH="$HOME/.cargo/bin:$PATH" cargo test --workspace`
Expected: PASS, except the known `live_tmux` flake.

```bash
git add -A
git commit -m "feat(claude): a native handshake over the stream-json control protocol

Flags and frame shapes read off vendor/claude-sdk.d.ts rather than
guessed. CLAUDECODE is scrubbed before spawning, for the reason the ACP
path already scrubs it."
```

---

## Self-Review

**Spec coverage.** Every section of the design maps to a task: crate layout →
Tasks 1, 3, 9, 10; the trait → Task 2; `Capabilities` → Tasks 2, 4;
`set_config_option` collapsing → Task 2; `steer` → Task 5; failure taxonomy →
Task 2; session identity and `replay` → Tasks 2, 4; the `backend` field and the
`args` meaning → Task 6; codegen → Task 8; tests → Tasks 5, 7, 9, 10; build
order → the task order itself.

**Not covered, and deliberately.** The spec's cross-backend parity test and
per-backend fixture captures need two backends that can hold a conversation.
Neither exists at the end of this plan — both native backends stop at a
handshake, which is what the spec's build order asks for. The parity test lands
with spec #2, and this plan's Task 9/10 live handshake tests are what guard the
seam until then. `AgentGapReason::LoadUnsupported` also stays reachable, since
ACP remains the only backend that runs a session.
