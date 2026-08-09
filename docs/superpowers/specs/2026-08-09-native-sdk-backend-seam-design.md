# Design: One seam, three backends

Chat mode speaks ACP to every agent it hosts. This cuts a seam underneath that,
so `claude` and `codex` can be driven through their own protocols while `cursor`,
`opencode`, and every config-added adapter keep speaking ACP unchanged.

This is the first of four specs. It ships no new feature. Its whole job is to
put the boundary in the right place, and to prove it against one real backend
before the expensive work lands on top.

## The problem

`crates/agent/src/lib.rs` already states the intent: "Clients never see a vendor
protocol. That is the entire point — the UI is written once against
`event::AgentEvent`, and a new agent is a new adapter rather than a new screen."

That is true of `event.rs`. It is not true of `session.rs`. `AgentSession` and
`RunningSession` hold an `AcpWriter`, call `acp::normalize::update_to_events`,
and wrap `AcpError` in `SessionError`. `crates/cli/src/agent_host.rs` constructs
an `AcpConnection` directly. The abstraction is one layer higher than the code
that implements it, so "a new agent is a new adapter" is currently only true for
agents that already speak ACP.

That matters now because ACP is costing real capability:

- **ACP is a floor, not a ceiling.** It has no subagent concept at all, which is
  why `acp/wire.rs` carries a `_meta.claudeCode` envelope to smuggle the
  structure through. Plan mode, context usage, file checkpointing, hooks, and
  per-turn effort have no expression in ACP 1.2 either.
- **The claude path is three hops deep.** `@agentclientprotocol/claude-agent-acp`
  (0.66.0) depends on `@anthropic-ai/claude-agent-sdk` (pinned at 0.3.220 against
  a live 0.3.226), which spawns the `claude` binary. `npx` resolves and downloads
  before any of that starts, which is what the 90-second `Status::AdapterSilent`
  bound in `agent_host.rs` exists to tolerate.
- **`npx` is an undeclared prerequisite.** `docs/remote-hosts.md` promises two
  static musl binaries copied into `~/.local/bin`, with prerequisites listed as
  exactly tmux, git, and systemd. Node appears on no list, yet three of the four
  built-in adapters cannot start without it.

## What we are building

A `Backend` seam inside `crates/agent`, with ACP moved behind it unchanged, and
the crate split so that generated vendor types cannot drown the ~500 lines of
logic that matter.

### In scope

- A `farcooler-agent-event` crate holding the vendor-neutral vocabulary.
- `farcooler-acp` carrying today's ACP implementation, moved rather than rewritten.
- Empty `farcooler-claude` and `farcooler-codex` crates with their handshake,
  spawn, and capability reporting — enough for the drift guard to run against a
  real binary, and nothing more.
- A `Backend` enum in `farcooler-agent` and the neutral session logic that was
  previously tangled with ACP.
- A `backend` field on `AdapterSpec`, defaulting to `acp` for every preset.
- `scripts/regen-backend-types.sh`, and the generated types it produces, committed.

### Out of scope

- Any new `AgentEvent` variant. The seam ships with zero proto and zero app
  changes; that is what makes it safe to land first.
- Normalizing anything the ACP path does not already normalize. `farcooler-claude`
  and `farcooler-codex` can handshake and report a version. They cannot yet hold
  a conversation, and `backend = "native"` is therefore not the default for any
  preset when this lands.
- `cursor`. `@cursor/sdk` (1.0.27, published 2026-08-06) is first-party and
  would replace a third-party adapter stalled at 0.1.1, but it authenticates with
  `CURSOR_API_KEY` and bills tokens. Swapping a subscription-backed adapter for a
  metered one is a product decision, not a refactor, and it is deferred.
- `opencode`. `opencode acp` is a subcommand of the binary itself, with no npm
  package to go stale. It is already the good case and there is nothing to fix.

## Decisions, and why

### Both native backends are Rust, because the install contract says so

The obvious alternative was a Node shim per agent, written against each vendor's
official TypeScript SDK. Writing against a typed, supported API beats writing
against a wire format, and for a while that looked decisive.

It loses to `docs/remote-hosts.md`. "Everything installs into your home
directory. No root, no package manager, no system service." A shim means either
Node becomes a documented prerequisite — cementing something that is currently
an accident — or `host install` grows a second per-platform artifact to copy and
checksum. Both are a worse trade than the one thing the shim was buying.

And it was buying less than it appeared to. The Claude control protocol is not
documented in prose, but it is fully typed in an artifact that ships with the
SDK and versions with the CLI: `sdk.d.ts` is 7,453 lines and 205 exported types,
declaring `SDKMessage` as a union of 39 variants, 46 `SDKControl*` types (36
requests, 7 responses), and 31 hook events. Codex is better still — `codex
app-server generate-json-schema` emits the protocol from the exact binary
installed. Neither backend requires reverse engineering. Both require codegen.

### The wire types are generated, and the generator is not a build step

`scripts/regen-backend-types.sh` runs `codex app-server generate-json-schema`
against an installed codex and `npm pack @anthropic-ai/claude-agent-sdk` for
`sdk.d.ts`, regenerates both type sets, and the output is committed with its
pinned vendor version recorded beside it.

Committed rather than generated at build time, because `cargo build` must not
need the network, and CI must not need npm to compile a Rust workspace. The
versions this tree was generated against are a fact about the tree, and they
belong in it.

The generated surface is large and mostly unread. Measured against codex-cli
0.146.0, the schema declares 90 client requests, 70 server notifications, 10
server-to-client requests, and — in the v2 bundle, 537 definitions in total — 18
`ThreadItem` variants. Only about twenty of those methods matter to chat mode.
Generating all of them anyway is correct: a hand-curated subset is a list
somebody has to remember to update, and the test is what protects us, not the
curation.

### Backends are crates, dispatched by an enum

Three protocol implementations and two sets of generated types do not belong in
one crate. `crates/agent` is 4,300 lines today; folding the generated code in
would make it the largest thing in the workspace by an order of magnitude and
bury the logic inside it.

So each backend is a crate depending only on `farcooler-agent-event`. A backend
cannot reach another backend's types, or the orchestration above it, because the
dependency graph will not let it.

Dispatch is an enum rather than `dyn AgentBackend`. Async fn in traits is not
dyn-compatible, so a trait object would mean adding `async-trait` and boxing
every call. `crates/agent` has six dependencies in total, and the workspace
comment on `reqwest` — "the only HTTPS client in the workspace, and it exists for
exactly one call" — shows how deliberately a dependency gets added here. The
backend set is closed by design anyway: every config-added adapter is ACP, so
adding a backend is a code change regardless. The trait defines the shape, for
documentation and for the test fake. The enum does the work.

### The trait carries no `set_mode` and no `set_model`

The comment on `ConfigOption` in `event.rs` already argues this: the adapter
advertises a list of options and the client renders one control each, "rather
than the client knowing in advance that 'mode' and 'model' exist. That
genericness is the point."

The trait honors it. There is one `set_config_option(id, value)`, and `mode` and
`model` are well-known ids on it.

`proto/farcooler.proto` keeps `AgentSetMode`, `AgentSetModel`, and `AgentSetConfig`
as three separate messages, because shipped apps send them and that is a real
compatibility surface. `link.rs` collapses to one, because its header comment is
explicit that it has none: "the shim IS the daemon binary under another
subcommand, so the two can never disagree about a schema."

### `steer` is a capability, not a message

Far Cooler emulates mid-turn steering today by holding a prompt and flushing it
at `TurnEnded`, which is the honest thing to do when the protocol has no way to
inject into a running turn. Codex has `turn/steer` and does not need the
emulation.

So `steer` is a trait method whose default implementation is the existing queue,
overridden by Codex. `Capabilities.native_steer` is what lets the composer stop
implying a queued prompt was delivered when it is actually still sitting in a
queue. Same `DaemonMessage::SteerQueued` either way; only the truthfulness of the
UI changes.

### A backend that cannot start leaves a terminal

This is the existing contract for `Status::AdapterMissing` and
`Status::AdapterSilent`, and it is the right one: the pane keeps working as a
terminal, and one message names what failed. Automatic fallback to ACP was
considered and rejected — a quietly degraded transcript, with fewer item types
and no plan mode, is exactly the class of thing this project refuses elsewhere.
"When a terminal is expected to be alive and no live exactly-tagged pane proves
it, the answer is `LOST` rather than a guess."

`BackendError` therefore has five variants, four of which already exist under
other names:

| Variant | Today |
|---|---|
| `Spawn` | `Status::AdapterMissing` |
| `Silent` | `Status::AdapterSilent`, still on the 90-second bound |
| `Closed` | `AcpError::Closed` |
| `Refused(String)` | `AcpError::Refused`, still the agent's own words |
| `Incompatible { found, expected }` | new, and native-only |

`Incompatible` is what version skew looks like. A native backend handshakes
against a CLI whose protocol was generated from a pinned version; when the
installed binary reports something the pins do not cover, the pane says so:

> farcooler: this `codex` (0.152.0) speaks a newer app-server protocol than this
> Far Cooler was built against (0.146.0), so chat mode stays a terminal. Set
> `backend = "acp"` under `[adapters.codex]` to use the ACP adapter instead, or
> update Far Cooler. Terminal mode needs no backend and is unaffected.

The escape hatch is a config edit, which is the same shape as every other adapter
override, and the last line is the same reassurance the existing messages carry.

## Architecture

### Crate layout

```
farcooler-agent-event      AgentEvent, Seq, Sequenced, PromptImage, AgentChoice,
  (new)                    ConfigOption, Diff, PlanEntry, QueuedPrompt,
                           Capabilities, BackendError. serde only.
        ▲
        ├── farcooler-acp      wire types, AcpConnection, normalize, handshake
        ├── farcooler-claude   generated stream-json types, spawn, handshake
        ├── farcooler-codex    generated app-server types, spawn, handshake
        ▲
farcooler-agent            ring.rs, link.rs, fs_guard.rs, activity_source.rs,
  (shrinks)                the neutral session logic, and the Backend enum
```

Workspace members go from 11 to 15.

### The trait

```rust
pub trait AgentBackend: Send {
    fn capabilities(&self) -> Capabilities;
    async fn prompt(&mut self, text: &str, images: &[PromptImage]) -> Result<(), BackendError>;
    async fn steer(&mut self, text: &str) -> Result<(), BackendError>;
    async fn answer(&mut self, request_id: &str, option_id: &str) -> Result<(), BackendError>;
    async fn set_config_option(&mut self, id: &str, value: &str) -> Result<(), BackendError>;
    async fn cancel(&mut self) -> Result<(), BackendError>;
    async fn next_events(&mut self) -> Result<Vec<AgentEvent>, BackendError>;
}
```

`next_events` blocks until the backend has something to say and never returns an
empty vector, which is what keeps the caller from spinning.

### Capabilities are behavioral only

```rust
pub struct Capabilities {
    pub backend: BackendKind,   // Acp | Claude | Codex — for messages and logs
    pub native_steer: bool,     // turn/steer exists; otherwise the queue emulates it
    pub replay: bool,           // can rejoin a session it did not start
    pub client_side_fs: bool,   // sends fs/* requests back, so paths need confining
}
```

Modes, models, and config options are deliberately absent. They are not
capabilities — they arrive dynamically on `SessionStarted`, which is already how
`modes_from`, `models_from`, and `config_options_from` read them.

`client_side_fs` exists because ACP and Codex ask the client to perform file
operations and therefore need `fs_guard::confine`, while the Claude CLI does its
own file IO and never asks. Confinement stays in the neutral layer and is applied
only where a backend declares it necessary.

### Session identity

Every claude and codex terminal is already launched with a `--session-id`, and
chat mode joins that same id. What changes is the mechanism:

| Backend | Joins by | A failure surfaces as |
|---|---|---|
| ACP | `session/load` | one of the three `AgentGapReason` variants |
| Claude | `--resume <id>` at spawn | a spawn failure, before a transcript exists |
| Codex | `thread/resume` after `initialize` | a handshake failure |

`Capabilities.replay` is what the neutral layer reads to decide whether to
attempt a rejoin at all. An ACP adapter that declared no `session/load` support
at `initialize` reports `replay: false`, and the layer emits
`AgentGapReason::LoadUnsupported` without making a request it knows will fail —
which is what that variant already means today, decided one layer lower.

Both native backends report `replay: true`, so `LoadUnsupported` becomes
unreachable for them. It stays in the enum for ACP. Resume also moves from a
mid-session request to a launch-time fact, which is a better place for it to
fail.

### What native does not fix

`session_discovery.rs` keeps globbing `~/.claude/projects/*.jsonl` and
`~/.codex/sessions/**/rollout-*.jsonl`. That probe runs in the daemon to decide
whether a *terminal* pane is launched with `--resume`, and neither CLI offers a
cheap replacement: `claude` has no session-list subcommand, and asking codex
would mean starting an app-server purely to ask it. Native improves the chat
path. The terminal-launch probe stays exactly as it is.

### The `backend` field

```toml
[adapters.codex]
backend = "native"          # defaults to "acp" for every preset
program = "/opt/codex/bin/codex"   # optional: a custom install path
args = ["-c", "model=o3"]          # optional: appended AFTER the protocol flags
```

Note what is absent. `app-server` does not appear, because it is not a
preference — it is how the backend talks, and the backend owns it.

Read at daemon startup with the rest of `[adapters.*]`, per the existing rule
that the registry is held for the life of the process — so changing it needs
`farcooler daemon ensure`, which is documented behavior rather than a new
surprise.

One wrinkle worth stating, because it changes what a hand-edited file means.
`AdapterSpec.args` is the complete argument vector today. Under
`backend = "native"` the protocol flags belong to the backend, not to config, so
`args` becomes *extra* arguments appended after them. `program` stays overridable
for a custom install path. `env` is unchanged.

That also changes the `Test` button in the machine-settings editor: it performs
the handshake for the configured backend rather than an ACP handshake. It still
proves only the launch half, and the form should keep saying so.

## Tests

**Fixtures per backend, captured rather than written.** Each backend crate gets
`tests/fixtures/*.jsonl` recorded from a real session, with a normalizer test
asserting frames become a `Vec<AgentEvent>` — the shape `normalize_fixtures.rs`
and `v064_fixture.rs` already use. `capture_subagent.py` generalizes into a
per-backend capture script. Fixtures stay recordings, under the same rule the
`identity` table lives by: read off a running instance, never guessed.

**One cross-backend parity test.** A scripted conversation — prompt, tool call,
permission request, turn end — captured through each backend, asserting the same
`AgentEvent` variants arrive in the same order. Payloads differ because the
agents differ. The invariant is that no backend emits a `Gap` where another emits
content. This is the test expected to catch an unmapped item type once the three
normalizers start drifting apart.

**The neutral layer, tested with no vendor at all.** A `Backend::Fake` variant
behind `#[cfg(test)]` makes the prompt queue, the steering emulation, ring
interaction, and path confinement directly testable. `properties.rs` calls these
"the three invariants the whole design rests on," and at present they can only be
exercised through a live `npx` subprocess.

**The drift guard, and the move it forces.**
`every_built_in_adapter_completes_an_acp_handshake` becomes
`every_built_in_backend_completes_a_handshake`, keeping the documented rule that
a missing program is a failure and not a skip — "on a machine without the agent
installed, silently passing would mean the one test that can catch a broken
adapter never runs where it matters."

It moves from `crates/core/tests/` to `crates/agent/tests/`, and the handshake
moves with it. `core::activity::handshake` cannot dispatch to the backend crates
without inverting the dependency graph, so each backend owns its handshake and
the enum dispatches. The `Test` button reaches it through the daemon, which
already depends on `farcooler-agent`. The property that mattered — that the
button and the test are one implementation — survives; only its address changes.

`no_built_in_adapter_is_deprecated_on_npm` stays as it is, and now covers only
the adapters that still run npm packages.

## Risks

- **`codex app-server` is marked `[experimental]`** in the CLI's own help output.
  Its schema moves between versions. This is the risk `Incompatible` and the
  drift guard exist for, and it is the reason `backend` defaults to `acp`.
- **The Claude control protocol has no compatibility promise**, only a typed
  artifact. `sdk.d.ts` is a description of what the CLI does today, not a
  contract about tomorrow.
- **Three normalizers instead of one.** `acp/normalize.rs` is a single 497-line
  file today. That property is being traded away deliberately, and the parity
  test is what is bought with it.
- **The seam could still be wrong.** It is cut against ACP's shape, which is the
  only backend fully understood right now. Building `farcooler-codex` far enough
  to handshake — rather than stubbing it — is what tests that before the
  transcript work commits to it.

## Build order

1. `farcooler-agent-event`, extracted from `event.rs`. Mechanical, no behavior.
2. `farcooler-acp`, moved from `crates/agent/src/acp/`. Still no behavior change;
   existing fixture tests must pass untouched.
3. The `Backend` enum, the trait, and `Capabilities`. `session.rs` splits into the
   neutral half and ACP's half. `Backend::Fake` and the neutral-layer tests.
4. `agent_host.rs` selects by preset; `backend` added to `AdapterSpec`, defaulting
   to `acp`. At this point everything behaves exactly as before.
5. `scripts/regen-backend-types.sh`, plus generated types for both vendors.
6. `farcooler-codex` to the point of a real handshake against a real binary, and
   the drift guard rewritten around it.
7. `farcooler-claude` to the same point.

Steps 1–4 are a refactor with no user-visible change and are individually
revertable. Steps 5–7 add code nothing dispatches to yet.

## References

- `docs/adapters.md` — the four built-in adapters, the config file, and detection.
- `docs/remote-hosts.md` — the install contract this design is constrained by.
- `docs/superpowers/specs/2026-08-01-native-agent-view-design.md` — chat mode's
  original design, and the reasoning for ACP.
- `docs/superpowers/specs/2026-08-02-acp-adapters-design.md` — the adapter registry.
- Codex app-server protocol: `codex app-server generate-json-schema`, measured
  here against codex-cli 0.146.0.
- Claude control protocol: `sdk.d.ts` in `@anthropic-ai/claude-agent-sdk`,
  measured here against 0.3.226.

### Specs that follow this one

2. **Transcript fidelity** — Codex's 18 `ThreadItem` variants and Claude's
   thinking, tool, and subagent shapes, as new `AgentEvent` variants and row
   renderers. Where most of the visible improvement lives.
3. **Session control** — resume, fork, rollback, compact, checkpoint and rewind,
   interrupt, and per-turn model and effort. Mostly rides the `ConfigOption` rail.
4. **Ambient surfaces** — token usage, rate limits, MCP server health, skills and
   plugins, account state, and fuzzy file search for @-mentions.

2 and 3 are independent of each other. 4 depends on 2's plumbing.
