# Design: More than one agent in chat mode

Date: 2026-08-02
Status: APPROVED (design); implementation not started
Extends `docs/superpowers/specs/2026-08-01-native-agent-view-design.md`.

## Problem

Chat mode speaks to Claude Code and nothing else. The pane knows better — a
codex pane is recognised, labelled `codex`, and reports live activity — but
`⌃B a` on it does nothing, because `CHAT_CAPABLE` in `crates/daemon/src/service.rs`
holds one entry.

That refusal is correct. `service.rs:1203` records why: the shim picks the
adapter and only knows one, so a codex pane switched to chat did not render
codex, it started a brand new *Claude* session in the same worktree and drew
that instead. Losing a toggle is a small disappointment; being handed a
different agent wearing the same pane is a much larger one.

But the refusal is also invisible, and that is the bug users actually hit. The
Mac app catches the case at `ContentView.swift:886` and writes an explanation
into `client.lastError`, directly beneath a comment reading *"Never silent. A
keystroke that does nothing and says nothing is indistinguishable from a broken
feature."* Nothing renders it. `client.lastError` has exactly one reader —
`fleetPlaceholder` at `ContentView.swift:488` — which draws only while
`!client.hasLoaded`, i.e. before the fleet finishes loading. In normal use the
message is written and discarded, so the keystroke does nothing and says
nothing.

Underneath both sits a structural fault. Two lists must agree and are maintained
by hand, separately:

- `activity::RULES` in `crates/core` — which agents are *recognised* in a pane.
- `CHAT_CAPABLE` in `crates/daemon` — which agents can be *hosted* as a chat.

Nothing enforces their relationship. They already disagree, which is exactly
what a user experiences as "codex autodetection is broken". Adding three agents
to two hand-maintained lists makes the drift worse, not better.

## What we are building

Chat mode gains adapters for **codex**, **opencode** and **cursor** alongside
claude, and an escape hatch for anything else via a config file. The two lists
above become one.

### What is already generic

`crates/agent` — `conn.rs`, `session.rs`, `normalize.rs`, `wire.rs` — has no
Claude branching anywhere. `session.rs:298-309` already handles both shapes
adapters use to advertise modes and models (`modes`/`currentModeId` and the
generic `configOptions` list), and folds them into one code path. The protocol
work is done.

Claude-specificity is confined to three places, and this design touches only
those: `default_adapter()` and `claude_executable()` in
`crates/cli/src/agent_host.rs`, and `CHAT_CAPABLE` in `crates/daemon/src/service.rs`.

### Out of scope

A Settings pane for adapters. The config file is the source of truth and is
hand-edited; a UI over it can come later without changing anything here.
Gemini CLI, Goose, Qwen, Copilot and the rest of the registry's forty-plus
entries — reachable through the escape hatch, not shipped as built-ins.

## The registry

One table, in `crates/core`, which `daemon` and `cli` already depend on and
which itself depends only on `protocol`. `AgentRules` gains one field:

```rust
pub struct AgentRules {
    pub preset:   String,
    pub commands: Vec<String>,
    pub identity: Vec<String>,
    pub blocked:  Vec<String>,
    pub working:  Vec<String>,
    pub adapter:  Option<AdapterSpec>,   // NEW
}

pub struct AdapterSpec {
    pub program: String,
    pub args:    Vec<String>,
    pub env:     Vec<(String, String)>,
}
```

`chat_capable()` stops being a list and becomes derived — an agent is chat
capable exactly when its entry carries an adapter. `service.rs:129`,
`service.rs:809` and `watch.rs:328` all collapse onto that one question. The two
lists can no longer disagree because there is only one list.

### Owned, not `&'static`

The fields become owned because the config file can extend and override the
table at runtime, which `&'static [&'static str]` cannot express. `RULES`
therefore stops being a `const` and becomes a function returning the built-in
table — a `const` cannot hold a `String`. Every current reader goes through
`identify`, `classify`, `describe` or `rules_for_command`, so `RULES` itself has
no callers to migrate.

`crates/core` gains two dependencies for this: `serde`, already in the
workspace, and `toml`, which is not and must be added to it.

The resolved table is a `Registry` **value**, not a process global. `Service`
holds one, exactly as it already holds `root` — and for the reason recorded at
`service.rs:166-172`: process-global state can be moved out from under a service
that re-reads it, which is what happens when two tests run in parallel. There
are seven call sites, all in `service.rs` and `watch.rs`, so threading a value
is cheap. The shim builds its own from the same loader.

### The built-ins

Verified against npm on 2026-08-02, not taken from documentation:

| preset | program | args | note |
|---|---|---|---|
| claude | `npx` | `-y @agentclientprotocol/claude-agent-acp` | 0.64.2; unchanged from today |
| codex | `npx` | `-y @agentclientprotocol/codex-acp` | 1.1.9 |
| opencode | `opencode` | `acp` | native subcommand; no npm dependency |
| cursor | `npx` | `-y cursor-agent-acp` | 0.1.1, best-effort — see below |

Two traps this table avoids. `@zed-industries/codex-acp` is what every current
search result and third-party doc names for codex; npm reports it **deprecated**
— *"replaced by `@agentclientprotocol/codex-acp`"* — and stalled at 0.16.0 while
the live package is at 1.1.9. This is the identical failure `agent_host.rs:61`
already records for the Claude adapter: same scope, same 0.16.x dead end. The
pattern is that `@zed-industries/*` ACP packages have all moved to
`@agentclientprotocol/*`.

Neither `codex` nor `cursor-agent` has a native ACP mode; both were checked
directly. `opencode acp` is native, which makes opencode the only one of the
three with nothing to go stale.

**cursor is shipped flagged.** Its only adapter, `cursor-agent-acp`, is
third-party, sits at v0.1.1, and was last published 2025-09-03 — roughly eleven
months stale. It needs no special code: the existing `AdapterMissing` and
`AdapterSilent` states in `agent_host.rs:30-57` keep the pane alive with
readable text when an adapter cannot start or never answers, which is precisely
the degradation cursor may need. What it does need is a comment on its registry
entry recording that the adapter is unmaintained and why it is shipped anyway —
the same way `activity.rs:114` records that its detection rules were unverified,
so the next person to touch it does not have to rediscover this.

`claude_executable()` stays as it is and applies to the claude preset alone. It
probes the filesystem at runtime, so it cannot become a static config value, and
generalising it before a second adapter needs the same treatment would be
invention.

## Configuration

`~/.config/farcooler/config.toml`, on macOS as well as Linux.

Deliberately **not** `ProjectDirs::config_dir()`. On Linux that resolves to
`~/.config/farcooler`, but on macOS it returns the same directory as
`data_dir()` — Apple does not separate config from data — so it would achieve
the separation only on the platform where the file is least likely to be edited
by hand.

Deliberately **not** inside `FARCOOLER_HOME` either. Everything there is
machine-generated state: `farcooler.db`, sockets, `install-id`, `worktrees`, in
a directory chmod'd 0700. A hand-edited file does not belong in it — nobody
browses Application Support, dotfiles repositories do not track it, and deleting
state to recover from a corrupt database would take the config with it. The
project already writes to `~/.config` on Linux hosts at `host_install.rs:163`.

One path on every platform means one dotfiles-tracked config works on the Mac
and on every remote host, which matters because `docs/remote-hosts.md` hosts run
`farcoolerd` with no Mac app anywhere near them.

Resolution order: `$FARCOOLER_CONFIG` → `$XDG_CONFIG_HOME/farcooler/config.toml`
→ `~/.config/farcooler/config.toml`. A missing file is not an error; the
built-ins are the defaults and no file at all is the common case. A malformed
file **is** an error, reported and then ignored in favour of the built-ins — a
typo must not take chat mode down for every agent.

```toml
# Override a built-in: pin codex rather than tracking latest.
[adapters.codex]
program = "npx"
args = ["-y", "@agentclientprotocol/codex-acp@1.1.9"]

# Add an agent Far Cooler has never heard of.
[adapters.my-agent]
program = "node"
args = ["~/src/agent/index.js", "--acp"]
identity = ["My Agent v"]
env = { MY_TOKEN = "..." }
```

Merge is by preset name: a table naming a built-in replaces its adapter, a table
naming anything else adds a new entry.

An added entry may also carry `commands` and `identity`, and for a genuinely new
agent it must. `⌃B a` selects the adapter from the preset the pane was
*detected* as, so an adapter belonging to a preset nothing can ever detect is
dead config. Following `activity.rs:50`, `identity` strings must be furniture
the agent always draws and never a phrase a user could type, or a shell echoing
the wrong thing gets promoted to an agent.

## Data flow

Unchanged in shape from today; only the lookup in step 2 is new.

1. `activity::identify(command, screen)` resolves the pane to a preset, by
   process name first and screen furniture second — Claude Code renames itself
   to its version number, so process matching alone will never find it.
2. `registry.adapter(preset)` is `Some` or `None`. That answer is what
   `chat_capable` puts on the wire, and what the Mac app reads as
   `canSwitchPaneMode`.
3. `set_pane_mode(Agent)` refuses when it is `None`, with a message naming the
   agent.
4. Otherwise the daemon respawns the pane running the shim, passing
   **`--preset codex`** rather than a resolved program and argument list.

Step 4 is the one real change to the spawn path. The shim resolves the preset
through the same registry and the same config file the daemon used. One
resolution path rather than two that can disagree — and, given the shell-quoting
already required at `service.rs:1162` to survive a worktree path containing a
space, one fewer thing to quote through a tmux command string.

`--adapter` on `agent-host` is replaced by `--preset`. Nothing currently passes
`--adapter`; the daemon never set it, so there is no compatibility surface.

## Error handling

Three failures, three distinct messages, none of them silence.

**No adapter for this agent.** Refused at `set_pane_mode`, and refused earlier
and more helpfully in the Mac app, which knows the agent's name. The pane is
untouched and terminal mode is unaffected.

**The adapter will not start.** `AdapterMissing` — the pane stays alive and
prints how to install it, and says that terminal mode needs no adapter. Already
implemented; it now names the agent whose adapter is missing.

**The adapter starts and never answers.** `AdapterSilent`, after the existing
90-second bound, which is generous because a cold `npx` genuinely has to fetch a
package. Already implemented.

And the bug that prompted this work: **a written error nothing renders.** Add
the missing renderer for `client.lastError` so the three call sites that write
it are actually seen. Also delete the stale `a` and `⇧T` rows from
`Shortcuts.swift:62-63` — `a` is listed twice in the same group, and the first
listing describes a binding `Prefix.swift:204` records as removed.

## Testing

**Unit.** Registry lookup; config merge (override a built-in, add an entry,
malformed file falls back to built-ins); `chat_capable` derived rather than
listed.

**A drift test.** Every preset carrying an adapter must be reachable by
`identify`. This is the test that makes the original class of bug impossible
rather than merely fixed, and it is the reason the two lists became one.

**Detection rules read off real screens.** `activity.rs:67` records that the
first version of that file was guesswork and matched no real screen, and
`activity.rs:114` still marks the cursor rules UNVERIFIED because that install
could not get past sign-in. codex, cursor-agent, opencode and claude are all
installed on this machine, so opencode's rules get read off a running agent and
cursor's finally get verified rather than assumed.

**Integration, one per built-in.** Spawn the adapter for real and assert that
`initialize` and `session/new` complete. This is the test that would have caught
the deprecated `@zed-industries/codex-acp` before it shipped, and the only kind
that can. Marked `#[ignore]` so a machine without the CLIs installed still has a
green suite, and run deliberately.
