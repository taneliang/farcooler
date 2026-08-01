# Design: A native agent view, alongside the terminal

Date: 2026-08-01
Status: APPROVED (design); implementation not started
Supersedes nothing. Extends `docs/overnight-design.md`.

## Problem

Overnight renders coding agents as what they are: a TUI in a tmux pane. That is
honest and it is the product's constraint — terminal-first on both desktop and
mobile — but a phone screen full of ANSI is a poor way to answer the question
the user actually has at 3am, which is "what did it do, and does it need me?"

A native rendering of the same session can answer that far better: messages as
messages, tool calls as rows with status, edits as diffs, and an approval as a
card with buttons rather than a wall of box-drawing characters. What it must not
do is take the terminal away, or lie about a session it can only partially see.

## What we are building

A terminal in Overnight gains a **pane mode**. In `terminal` pane mode it behaves
exactly as it does today. In `agent` pane mode the same pane hosts a headless
coding agent speaking the Agent Client Protocol, and clients render a native chat
instead of a VT grid.

Terminal mode remains the default and the entry point. Agent mode is an upgrade
*offered* on a terminal where Overnight can already see a supported agent
running — the signal `activity.rs` produces today. A future preference may take
the offer automatically; nothing in this design needs to change for that.

On the Mac an agent pane is an ordinary pane: it occupies a real tmux rectangle
and participates in tiling, dividers, focus navigation and zoom without any new
layout code. Several native agent chats tiled alongside terminals is therefore
the default capability, not a separate feature. On iOS an agent view is a
full-screen view swapped with the terminal view behind the existing tab strip.

### In scope for v1

Transcript of user and agent messages; thinking collapsed behind a disclosure;
tool calls as rows with live status; inline unified diffs for edits; a live plan
panel; structured permission requests as approval cards; slash commands;
`@`-mention file picker over the worktree; image upload; and the agent mode
switcher (plan / accept-edits / default).

### Out of scope

Per-hunk accept/reject before a write lands. Checkpoint and rewind. Syntax
highlighting inside diffs (a later, isolated upgrade to one view). Agents other
than Claude Code. Cross-session search.

## Decisions, and why

### The agent is driven, not observed

Two architectures were considered.

**Observe:** leave the TUI running and derive a native view by tailing the
agent's on-disk transcript. Toggling would be free and simultaneous, and the
terminal would remain literally the only runtime. Rejected because permission
requests do not appear in those transcripts until they are resolved, so the one
state that justifies the whole feature — *this agent is blocked and needs you* —
would still have to come from screen-scraping, and because the on-disk format is
explicitly internal and changes between releases.

**Drive (chosen):** run the agent headless under ACP. Permission requests,
tool-call status, plans and modes are protocol features rather than inferences.
The cost is that terminal mode and agent mode are two processes, so switching is
a restart at a turn boundary rather than a free flip.

### ACP, via Zed's adapter

Claude Code has no native ACP: `claude --help` exposes no such flag, and the ACP
registry lists it as "Claude Agent (via Zed's SDK adapter)", i.e. the Node
package `@zed-industries/claude-code-acp`. The alternative was Claude's own
`--print --output-format stream-json --input-format stream-json`, which is
first-party and needs no Node.

ACP was chosen for one reason: the client-side `fs` capability. With it,
`fs/read_text_file` and `fs/write_text_file` route through Overnight's own shim
instead of the agent touching disk, so edits arrive as before-and-after and diffs
are a protocol fact. Without it, diffs must
be reconstructed from tool-call arguments — precisely the coupling to vendor
internals that the terminal-first approach exists to avoid. A Node dependency is
an installation-document problem; a missing `fs` capability is an architectural
one.

Shipping both adapters was rejected: a fallback that is structurally less
capable than the primary means designing the degraded UI twice.

Choosing ACP also buys Codex and Cursor later as adapter configuration rather
than adapter code. Cursor has native ACP; Codex is adapter-based.

### The agent runs in a tmux pane

Overnight's central invariant is that runtime state is derived, never stored:
SQLite holds intent, tmux is the sole authority on liveness, and there is no
column where a stale `running` could be written. A daemon-owned child process
would be exactly such a place.

So an agent-mode pane runs `overnight agent-host`, a shim that spawns the real
ACP adapter with clean pipes and bridges the protocol to a Unix socket. The pane
displays a plain human-readable status log. `derive.rs` is untouched: a live
exactly-tagged pane means running, its absence means `LOST`, and `overnight
attach` still works. A crashed agent surfaces through the path already built and
already trusted.

The visible cost: while in agent mode, "show me the terminal" shows a protocol
log rather than a TUI. The TUI appears after switching to terminal mode. This is
accepted as more honest than the alternative.

### Pane mode is a property of a terminal

`Terminal` gains `pane_mode` and `agent_session_id` rather than a new
`AgentSession` resource being introduced beside it. The session id is durable intent on a
terminal row, which is what the SQLite/tmux split is for, and which
`docs/overnight-design.md` already anticipates by listing agent session IDs
among the things SQLite stores.

The consequence is that the tab strip, tiles, sidebar rows, activity badges,
notifications, visit log, shortcuts and drag-and-drop all keep working with no
new concepts, and the feature ships as a renderer plus a launch mode.

The trade-off accepted: a workspace cannot hold a background agent with no tab.
If that becomes wanted, the field can be promoted into its own table later.

### The conversation is derived, never stored

Overnight persists no transcript. SQLite stores only the session id, as intent.

The buffer lives in the **shim**, not the daemon: the shim is co-located with the
agent, lives exactly as long as the pane whose liveness is already authoritative,
and dies with it. The daemon listens on a per-session socket and the shim dials
out with retry, so a daemon restart does not disturb the agent — the shim
reconnects and replays from the daemon's last cursor. The daemon keeps a small
recent window for fast client attach and reuses the existing fanout.

`session/load` is therefore needed for exactly one case: toggling back into agent
mode after time spent in terminal mode. It is an optional ACP capability; an
agent lacking it produces a `Gap` rather than a silent hole.

### Naming

Three unrelated things in one pane would otherwise all be called "mode", so each
gets a distinct name and the bare word is never used alone:

- `mode` — a terminal's own VT modes. Unchanged; already in the codebase.
- `pane_mode` — whether the pane hosts a TUI or an ACP agent. New here.
- `agent_mode` — the ACP concept (plan, accept-edits, and whatever else the agent
  advertises in `available_modes`). New here.

## Architecture

```
tmux pane (tagged; liveness derived exactly as today)
  └─ overnight agent-host  ── spawns ACP adapter, owns its stdio
        │  owns the bounded event ring; dials the daemon's socket with retry
        ▼
   adapter (ACP)  ──►  normalizer  ──►  daemon fanout  ──►  clients
```

One new crate, `crates/agent`: the normalized event model and the adapters.
Clients never see a vendor protocol.

### Normalized event model

```rust
pub enum AgentEvent {
    SessionStarted { session_id, agent_mode, available_modes, available_commands },
    Message   { role: User | Agent | Thought, text },
    ToolCall  { id, title, kind, status, locations },
    ToolUpdate{ id, status, content, diff: Option<Diff> },
    Plan      { entries: Vec<PlanEntry> },
    Permission{ id, tool_call, options: Vec<PermissionOption> },
    Resolved  { id, chosen: String },
    ModeSet   { agent_mode },
    TurnEnded { reason: EndTurn | Cancelled | Refusal | MaxTokens },
    Gap       { reason },
}
```

Every event carries a monotonic per-session `seq`. Clients subscribe from a
cursor, exactly as they do for terminal bytes.

`Gap` is load-bearing and is the same idea as `LOST`. It is emitted when the ring
trims, when the adapter cannot parse an update, and when a reconnected session
cannot be replayed because the agent does not implement `session/load`. The
native view renders it as a visible break in the transcript. A derived transcript
is only permissible in this product because it can say where it is incomplete.

### Protocol changes (`proto/overnight.proto`)

- `Terminal` gains `PaneMode pane_mode` (`TERMINAL` | `AGENT`) and
  `string agent_session_id`.
- New: `AgentSubscribe { terminal_id, from_seq }`.
- New: `AgentPrompt { terminal_id, blocks }` where blocks carry text, file
  mentions and images.
- New: `AgentAnswer { terminal_id, request_id, option_id }`.
- New: `AgentSetMode { terminal_id, agent_mode }`, `AgentCancel { terminal_id }`.
- New: `SetPaneMode { terminal_id, pane_mode }` — the toggle.
- New: a worktree file-search RPC backing the `@`-mention picker.
- `Event` gains an agent-event variant.

### Activity gets a better source

In agent pane mode, `AgentActivity` comes from the protocol rather than the
screen: a
turn in flight is `Working`, a pending `Permission` is `Blocked`, `TurnEnded` is
`Idle`. `advance()`, `seen()` and `wants_attention()` in `activity.rs` are reused
unchanged, so `Done` still means finished-and-unseen and notifications fire on
the same rule.

The screen classifier must **not** be consulted for agent-mode panes: it would
inspect the shim's status log, find no identity markers and report `None`.
Activity source is therefore selected by mode.

This makes the blocked signal exact rather than a generous list of hopeful
substrings, which is the largest reliability gain in this work — a missed
approval is the failure the notification story exists to prevent. It also takes
the UNVERIFIED `cursor` rules in `activity.rs` off the critical path once that
agent moves to ACP.

### The toggle

Implemented with `tmux respawn-pane -k -t <pane>`, which replaces the process in
place. Pane id, tag, rectangle and the whole layout survive, so a chat that is
one tile of four does not reflow the window. Kill-and-create would lose the
pane's position and is not used.

- agent → terminal: respawn with `claude --resume <session_id>`.
- terminal → agent: respawn with `overnight agent-host --terminal <id>
  --socket <path> --session <uuid>`.

`claude --resume` cannot attach to a turn already in flight. Toggling to terminal
mode mid-turn must report this and offer to cancel the turn first, never discard
it silently.

Because the ring is keyed by session id rather than by pane, it outlives the pane
the toggle replaces. Turns taken in terminal mode are invisible to the daemon at
the time; toggling back calls `session/load` and the agent re-emits its own
history, so the transcript heals across a round trip.

### Session identity

`claude --session-id <uuid>` exists, so every claude terminal Overnight launches
is given a uuid Overnight chose, stored in SQLite as intent beside the branch and
preset. Adoption of those terminals is exact.

Only a hand-typed `claude` needs discovery: the newest `.jsonl` under
`~/.claude/projects/<munged-worktree>/`, cross-checked against the pane's start
time. A workspace is one worktree, so that directory almost always holds one
session. Where it is ambiguous, Overnight refuses and names the candidate
sessions rather than choosing — attaching to the wrong conversation is worse than
declining to offer.

### Path confinement (security)

The shim is the ACP client, so the shim answers `fs/read_text_file` and
`fs/write_text_file`. It reads the old contents, performs the write, and emits the
before-and-after as a `ToolUpdate` carrying a `Diff`. The daemon performs no file
writes on an agent's behalf.

This means Overnight writes files on an agent's instruction, on a host reachable
from a phone. Every such path must be fully resolved — symlinks included — and
validated as inside the worktree the shim was launched for, and a rejection must
be surfaced to the user rather than swallowed. Without this the capability is an
arbitrary-write primitive on the host.

## Clients

### Shared Swift package

The transcript reducer, normalized event decoding, the permission state machine,
the diff model and the composer's parse of `/` and `@` go into one small SPM
package used by both apps. Views stay per-platform.

The two apps today duplicate `Model.swift`, `VTCore.swift` and the terminal
session types by copy. This feature's shared logic is a substantially larger body
of code than those, and two copies would drift in exactly the way that makes a
phone and a Mac disagree about one session. The package is scoped to what this
feature needs; the existing duplication is left alone.

### macOS

`AgentSurface` renders where `TerminalSurface` renders — same `paneCard()`, same
rect derived from tmux, same view-identity rules. `TileView` switches on
`terminal.paneMode` and computes no new arrangement.

Chrome: `TerminalPane` deliberately has no header and no footer, and this does
not reintroduce one. The composer row carries what belongs to composing — agent
mode, attach image, send. The terminal↔chat toggle lives with the other pane
commands: a `⌃B` binding, the command palette, and the pane context menu.

`CommandPalette`, `PaletteField` and `PaletteIndex` back both `/` (fed by
`available_commands`) and `@` (fed by the worktree file-search RPC). Approval
cards render inline in the transcript with the ACP options as buttons, bound to
`1`/`2`/`3` and return so muscle memory from the TUI transfers.

Two details that must be planned, not discovered:

- The `⌃B` prefix and `⌃L` navigation must keep working while a SwiftUI text
  field is first responder, which requires an explicit local key monitor.
- An agent pane must still report honest cell dimensions through `onViewport`, or
  tmux lays the window out against stale numbers and every terminal tile beside
  it is sized wrongly.

Diffs render as unified diffs with add/remove backgrounds, line numbers and
collapse-by-default for large hunks. No syntax highlighting in v1.

### iOS

The same surface without tiling: a full-screen agent view swapped with the
terminal view behind the existing `TerminalTabStrip`; composer over the keyboard
using the existing key-row work; images via the photo picker and paste.

## Verification

### Gate 1 — PASSED 2026-08-01

**Result: all four conditions passed.** Full evidence in
`docs/superpowers/specs/2026-08-01-gate1-acp-findings.md`.

The decisive one: the ACP adapter's `sessionId` IS a Claude Code session id.
`claude --resume <sessionId>` opens the same conversation, and a `session/load`
replay includes turns that were taken through the native CLI in between — so
ACP and `claude --resume` read one transcript file, not two stores. No
id-recovery fallback is needed, and the toggle's correctness argument holds as
designed.

Three corrections came out of it, all now fixed in the implementation:

- **`stopReason` arrives on the `session/prompt` response**, not as a
  `session/update`. Sending the prompt as a notification meant `TurnEnded`
  never fired — activity would have stayed `Working` forever, `Done` would
  never happen, and nothing would ever notify anyone.
- **`availableModes` lives in the `session/new` / `session/load` result**, not
  in `initialize`. Reading it from `initialize` yielded an empty list forever.
- **The project directory is munged from the RESOLVED cwd.** On macOS `/tmp` is
  a symlink to `/private/tmp`, so session discovery must canonicalize first.

Also found: `available_commands_update` is a real update kind, fires once per
turn carrying tens of KB, and needed an explicit variant — without one it
became a `Gap` and drew a spurious "history missing" break every turn.

The original statement of the gate is kept below, because it is why the spike
was run first and what would have happened had it failed.

Everything rested on one unverified assumption: that the ACP adapter's
`sessionId` maps to Claude Code's own session id. If it did not, `claude
--resume` in terminal mode would open a different conversation than the chat
view showed, and the toggle would silently display the wrong history.

Stand up the shim with `@zed-industries/claude-code-acp`, declare the `fs`
capability, and run one turn that edits a file. Confirm all four:

1. Writes arrive through `fs/write_text_file` rather than landing on disk
   directly.
2. Permission requests arrive as `session/request_permission`.
3. `session/load` replays a prior session.
4. The session id resolves to a file under `~/.claude/projects/<munged-worktree>/`
   that `claude --resume` accepts.

If (4) fails, the fallback is to record the mapping observed when the session
first appears on disk. Do not write UI before this is known.

### Tests

- Captured ACP sessions become fixtures replayed into the normalizer, so CI
  asserts normalized output with no live agent and no credentials.
- Agent events run through the existing contract suite unchanged across the
  in-memory, Unix-socket and SSH-stdio adapters. This is non-negotiable: the
  phone is the SSH path.
- Properties: `seq` is monotonic; replay from any cursor reconstructs an
  identical transcript; dropped history always yields a `Gap` rather than a
  shorter transcript.
- Path-confinement tests for `fs/*`, including symlink and `..` escapes.
- Live-agent tests sit in the separate authenticated lane where the vendor-CLI
  tests already run.

## Risks

- A third-party npm package sits in the supervised path. Pin the version; the
  shim must fail loudly and readably into its own pane when it cannot start.
- ACP capabilities are optional, so the UI needs per-capability degradation even
  with a single agent.
- Shim ring memory on a long overnight session; the ring is bounded and trimming
  is visible as a `Gap`.
- Node becomes a requirement for agent mode on a product that currently needs
  only tmux and git. Terminal mode must remain fully functional without it.

## Build order

Each slice is independently reviewable.

1. Gate 1 spike. Stop if it fails.
2. `crates/agent`: normalized model, ACP adapter, fixture tests. No UI, no daemon.
3. `overnight agent-host`: pane hosting, socket, ring, cursor replay, and the
   `fs` capability with path confinement.
4. Daemon: `pane_mode` and `agent_session_id`; `SetPaneMode` via `respawn-pane`;
   declared session ids; adoption resolution with honest refusal; agent-event
   fanout; activity sourced by pane mode.
5. Protocol changes and contract tests across all three adapters.
6. Shared Swift package and Mac `AgentSurface`: transcript, approvals, diffs,
   plan.
7. Slash commands, `@`-mentions, images, agent mode switcher.
8. iOS.

Slices 2–4 are where the surprises live. Slices 6–8 are UI against a contract
that is settled by then.

This is more than one implementation plan's worth of work. Slices 1–5 form the
first plan and end at a settled, tested contract with no user-visible feature.
Slices 6–8 form a second plan written against that contract. Splitting there
rather than earlier means the second plan never has to revise the first.

## References

- Agent Client Protocol — <https://agentclientprotocol.com>
- ACP agent list — <https://agentclientprotocol.com/get-started/agents>
- Zed, external agents — <https://zed.dev/docs/ai/external-agents>
- Claude Code sessions — <https://code.claude.com/docs/en/sessions>
