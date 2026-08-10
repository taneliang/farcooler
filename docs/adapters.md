# Chat mode and its adapters

Every pane in Far Cooler starts as a terminal: a shell, or a coding agent's own
TUI running inside one, exactly as if you had typed the command yourself.
`⌃B a` (`Toggle the focused pane between terminal and chat` in the Mac app's
shortcut sheet) asks the daemon to flip the focused pane into a native chat
instead — messages, tool calls, and diffs rendered by Far Cooler's own surface
rather than scraped off a terminal screen.

Chat mode needs an **ACP adapter**: a small process that speaks the
[Agent Client Protocol](https://agentclientprotocol.com) on one side and
drives the real agent on the other. Far Cooler ships one for each agent it
recognizes out of the box. An agent with no adapter simply stays a terminal —
that is a real, supported state, not a bug. Pressing `⌃B a` on a pane Far
Cooler cannot chat-host says so and does nothing else:

> `<agent>` has no chat adapter, so it stays a terminal. Add one in
> `~/.config/farcooler/config.toml`.

## The four built-ins

These are the adapters `Registry::built_in()` ships, in
`crates/core/src/activity.rs`. Versions are what a real handshake reported on
2026-08-02 (`crates/core/tests/adapters.rs`, `every_built_in_adapter_completes_an_acp_handshake`):

| Preset | Launch command | Package | Verified version |
|---|---|---|---|
| `claude` | `npx -y @agentclientprotocol/claude-agent-acp` | npm | 0.64.2 |
| `codex` | `npx -y @agentclientprotocol/codex-acp` | npm | 1.1.9 |
| `opencode` | `opencode acp` | none — native subcommand | tested against opencode 1.18.11 |
| `cursor` | `npx -y cursor-agent-acp` | npm, third-party | 0.1.1 |

Three of the four are npm packages fetched through `npx -y` on first use,
which is why a cold start can take longer than a warm one — `npx` has to
resolve and download before the adapter ever sees `initialize`. `opencode` is
the exception: `opencode acp` is a subcommand of the `opencode` binary itself,
so there is no npm package to go stale, get renamed, or get deprecated out
from under it. That is a deliberate advantage, not an accident, and it is why
opencode is preferred over wrapping some third-party ACP shim for it, the way
`claude` and `codex` need to.

## Do not use `@zed-industries/*`

**If you are adding or changing an adapter, do not reach for anything under
the `@zed-industries/` npm scope.** Every package there has moved to
`@agentclientprotocol/*`, and most search results and third-party
documentation still point at the dead names — this is the single most likely
way to misconfigure a working setup.

The concrete case: `@zed-industries/codex-acp` is deprecated on npm and
stalled at version `0.16.0`, while the live, maintained package is
`@agentclientprotocol/codex-acp` at `1.1.9`. Far Cooler's own test suite
guards against regressing on this — `no_built_in_adapter_is_deprecated_on_npm`
in `crates/core/tests/adapters.rs` runs `npm view <package> deprecated`
against every built-in adapter and fails the build if any of them names a
deprecated package.

## cursor is best-effort

`cursor` is shipped, and it is shipped honestly as the weakest link in the
table. `cursor-agent-acp` is a **third-party** adapter — there is no
first-party Cursor package and no `@agentclientprotocol/cursor-acp` — at
version `0.1.1`, last published 2025-09-03. It did complete a real ACP
`initialize` handshake when tested on 2026-08-02, but "not deprecated" and
"actively maintained" are different claims, and nobody but its one
maintainer controls whether the next Cursor release breaks it.

It ships anyway because the alternative — refusing to offer cursor chat mode
at all — is a certain failure, and this is only a possible one. If it stops
working, the pane does not vanish or silently fall back to a shell; it stays
alive and prints one of two messages from `crates/cli/src/agent_host.rs`:

- **The adapter never started** (`Status::AdapterMissing`):
  > farcooler: could not start the ACP adapter for `cursor` (`npx -y cursor-agent-acp`).
  > Install it, or switch this terminal back to terminal mode — terminal mode
  > needs no adapter and is unaffected.

- **The adapter started but never answered `initialize`** (`Status::AdapterSilent`),
  after a 90-second wait generous enough not to kill a cold `npx` fetch mid-download:
  > farcooler: the ACP adapter for `cursor` (`npx -y cursor-agent-acp`) started
  > but never answered.
  > Nothing is wrong with this terminal — switch it back to terminal mode and
  > it will work as it always has.
  > One known cause: the Claude SDK refuses to launch inside another Claude
  > Code session, and neither answers nor exits. Check that the
  > daemon's environment has no CLAUDECODE variable set.

Both messages used to name only the *runner* (`npx`, identical for claude,
codex, and cursor) rather than the agent — harmless when Claude was the only
adapter, useless once there were four. `Status::AdapterMissing` and
`Status::AdapterSilent` now carry the preset and the full command
(`crates/cli/src/agent_host.rs`) so a failure names what actually failed.

Either way, terminal mode for that pane keeps working exactly as it did
before you tried the toggle.

## Editing it from an app

Everything below can be edited from a settings screen instead of an ssh session
and a text editor: **Machines → a machine → Settings** on the Mac, and
**Settings → Settings on this machine** on iOS and Android. Per machine, because
each one has its own file and "the branch prefix" is a different answer on each.

Three things about those writes are worth knowing, because they are what make it
safe to point a UI at a file people hand-edit:

- **Comments and layout survive.** A write goes through `toml_edit` and touches
  one table, so everything else in the file — including the comment you left
  explaining why an adapter is pinned — comes back byte for byte.
  `config::a_write_leaves_every_byte_outside_its_own_table_alone` asserts it on
  the whole file rather than on a parse of it.
- **A malformed file is refused, not overwritten.** The reader tolerates a broken
  file by ignoring it; a writer that overwrote one would turn a typo into lost
  work. Fix it by hand first.
- **Deleting is how you revert.** Removing a table restores whatever Far Cooler
  ships under that name, including a later improvement to it. The editors call
  that **Revert to Default** and it writes nothing back.

`Test`, in the agent editor, starts the adapter and completes an ACP handshake.
It proves the **launch** half — `program`, `args`, `env` — and cannot prove
detection: `commands`, `identity`, `blocked` and `working` are matched against
output only that agent produces, and a wrong one there does not fail, it stops
the agent being recognized. The form says so rather than showing one checkmark
over seven fields.

The CLI reaches the same methods: `farcooler settings show`,
`settings set-branch-prefix`, `adapter list`, `adapter test <preset>`,
`adapter delete`, `theme delete`. There is deliberately no hand-typed `adapter
set` — seven fields including four string arrays is a worse editor than the file
itself, and `$EDITOR ~/.config/farcooler/config.toml` is the right tool for that.
The apps write over `--json-stdin`, which is a machine channel rather than a
flag surface.

## The config file

`~/.config/farcooler/config.toml`, on macOS and Linux alike — deliberately
not a platform-specific config directory, because one path that works
identically on a Mac and on every remote host is what lets a dotfiles
repository track it once. `crates/core/src/config.rs` resolves the path in
this order:

1. `$FARCOOLER_CONFIG`, naming the file itself.
2. `$XDG_CONFIG_HOME/farcooler/config.toml`.
3. `~/.config/farcooler/config.toml`.

**A missing file is the common case, not a problem.** Almost nobody has one,
and the daemon does not log about its absence. **A malformed file is
reported and then ignored** — a typo in one `[adapters.*]` table must not
cost every other agent its chat mode, so `tracing::warn!` names the file and
the parse error, and the built-in table is used unchanged.

**When the file is read depends on which table you edited**, and the split is
deliberate rather than an oversight:

| Table | Read | Why |
|---|---|---|
| `[adapters.*]` | Once, at daemon startup | It becomes a `Registry` held for the life of the process |
| `[themes.*]` | On every `theme.list` | A few hundred bytes of TOML, in exchange for no restart |
| `[branches]` | On every `host.get` | Same, and a settings editor will write this table |

For `[adapters.*]`, `Service::open_in` (`crates/daemon/src/service.rs`) loads the
table into a `Registry` held for the life of the process — deliberately, per the
comment on `Service`'s `registry` field: the file and the environment that
locates it are process-global, and a service that consulted them on every call
could be moved out from under itself by another thread, exactly as `root` could.
Editing an adapter therefore does nothing until the daemon restarts. Run
`farcooler daemon ensure` to pick it up — it replaces a running daemon with one
built from the same source, which for a config-only edit is just a restart.

The other two tables are read per call, so editing them takes effect on the next
one. Themes are read that way because a colour is something you tune by looking
at it; branches because the machine-settings editor writes that table, and a
value cached at startup would not reflect its own writes.

## Branch names

```toml
[branches]
prefix = "elt/"
```

Prepended to a branch name derived from a task description, so a task called
"add authentication" becomes `elt/add-authentication`. The default is `feat/`.

`prefix = ""` opts out and produces a bare slug. An **absent** key and an
**empty** one are different answers: absent means "use the default", empty means
"no prefix", and collapsing the two would make opting out impossible.

Taken literally beyond trimming surrounding whitespace, so `elt-` works as well
as `elt/` — no slash is added or removed.

**A table, not a top-level key**, and that matters when you edit by hand: TOML
puts a bare top-level scalar written *below* `[themes.paper]` inside that table,
so a `prefix = "elt/"` appended to the end of an existing file would silently
become a theme's property and do nothing at all.

The prefix is applied by the **client**, not the daemon, because the task
composer shows you the branch it is about to create — a prefix added on the far
side would make that preview a lie. The daemon still validates the finished name
through `validate::branch_name`, which is the check that actually protects git.
It is carried to clients on `Host.settings.branch_prefix`, so all three apps and
the CLI agree with the machine the worktree is created on rather than each other.

A configured entry is a `[adapters.<name>]` table:

```toml
[adapters.my-agent]
program = "node"
args = ["/path/to/my-agent-acp.js", "--acp"]
env = { MY_AGENT_API_KEY = "..." }

# Detection — required for a genuinely new preset, see below.
identity = ["My Agent v"]
commands = ["my-agent"]
```

`program`, `args`, and `env` are the launch fields — `env` is a table
(`KEY = "value"` pairs), not a list, because that is what TOML actually
deserializes a `[adapters.x.env]`-shaped map into. Naming an existing
built-in preset (`claude`, `codex`, `opencode`, `cursor`) replaces its
adapter; naming anything else appends a new entry.

**A genuinely new preset must also supply `identity` or `commands`.** The
`⌃B a` toggle picks an adapter by the preset a pane was *detected* as running
— not by what it was launched with — so an adapter attached to a preset
nothing can ever detect could never be selected: nothing will ever identify
that pane as that preset in the first place, so its adapter can never be
reached. Nothing in `Registry::merge` rejects a config-added adapter that
omits both fields — the built-in table alone is required to carry them
(`every_adapter_belongs_to_an_agent_that_can_be_detected` in
`crates/core/src/activity.rs` checks only `Registry::built_in()`) — so an
adapter like this is dead config that starts up fine and simply never gets
chosen, not a rejected one. A blank `program`, by contrast, genuinely is
refused at merge time rather than accepted and left to fail silently at
launch (`crates/core/src/config.rs`,
`an_entry_with_no_program_is_refused_rather_than_launched`).

Overriding a built-in's `program`/`args`/`env` alone — say, to pin an exact
package version — leaves its `identity`, `commands`, `blocked`, and `working`
untouched, because TOML has no way to say "leave this field alone": `merge`
only overwrites a detection field the config file actually supplied.

## The native backend

An ACP adapter is not the only way Far Cooler can host a chat. For `claude` and
`codex` it can speak the agent's own protocol instead — the Claude CLI's
stream-json control protocol, and `codex app-server` — with no adapter process
and no `npx` in between. It is opt-in, off everywhere by default, and turned on
one agent at a time by a fourth key on the same `[adapters.*]` table:

```toml
[adapters.claude]
backend = "native"
program = "claude"

[adapters.codex]
backend = "native"
program = "codex"
```

**`program` has to be named even though it looks inheritable.** It is the one
key on that table with no serde default (`ConfigAdapter` in
`crates/core/src/config.rs`), so a table that omits it fails to parse the whole
file — and then *every* adapter, not just this one, falls back to its built-in
(`a_file_missing_a_required_key_fails_to_parse_and_built_ins_survive`). Name the
agent's own binary rather than `npx`.

**`args` means something different under `native`.** With `backend = "acp"` it
is the complete argument vector, as it has always been. Under `native` the
protocol flags belong to the backend — `app-server` for codex, the stream-json
flags for claude — and `args` is appended *after* them, so `args = ["--model",
"opus"]` pins a model without being able to unset `--output-format` and leave a
process nothing can talk to. See the comment on `AdapterSpec::args` in
`crates/core/src/activity.rs`, and
`extra_args_are_appended_after_the_protocol_flags` in
`crates/claude/src/handshake.rs`.

Detection survives the switch untouched: the four arrays left empty or omitted
still mean "leave the built-in's alone", which they have to, because `⌃B a`
picks an adapter by the preset a pane was *detected* as
(`switching_a_built_in_to_native_keeps_its_detection` in
`crates/core/src/config.rs`).

This is an `[adapters.*]` edit like any other, so **it does nothing until the
daemon restarts** — the table under "The config file" above says why, and
`farcooler daemon ensure` is the restart. Afterwards `farcooler adapter list`
prints the protocol each adapter speaks in a column of its own, and `farcooler
adapter test claude` runs the *native* handshake rather than the ACP one:
`dispatch::handshake` (`crates/agent/src/dispatch.rs`) picks by the same field
the pane does. A typo falls back rather than failing — `backend = "nativ"` reads
as `acp`, for the same reason a malformed file is ignored rather than fatal
(`AdapterBackend::parse`, and
`an_unknown_backend_name_falls_back_to_acp_rather_than_losing_the_adapter`).

**Only `claude` and `codex` have one.** `AdapterBackend::native_is_available_for`
is the whole list, and the comment on it gives the reasons: cursor has no
first-party protocol Far Cooler speaks, and opencode is already a native
subcommand behind ACP with nothing to gain — it is the one built-in with no npm
package in the way to begin with. An adapter you added yourself is always ACP,
because a new preset by definition has no backend compiled in for it. Asking for
`native` anywhere else is refused rather than quietly served ACP, which would
report a working adapter for a protocol nothing ever spoke to
(`an_agent_with_no_native_backend_says_so_rather_than_testing_acp` in
`crates/agent/src/dispatch.rs`):

> `opencode` has no native backend; set backend = "acp" under [adapters.opencode]

The Mac adapter editor shows its **Protocol** picker only for the two presets
that have one, on the same rule (`nativeIsAvailable` in
`apps/macos/Sources/FarCooler/MachineSettingsStore.swift`).

## What native buys

`Capabilities` in `crates/agent-core/src/backend.rs` is the honest summary.
Three behavioral flags, and deliberately nothing about modes or models — those
arrive dynamically on `SessionStarted` and are not capabilities:

| `Capabilities` | ACP | `claude` and `codex` natively |
|---|---|---|
| `native_steer` | false | true |
| `replay` | per connection, whatever `initialize` advertised | true |
| `client_side_fs` | true | false |

**Steering.** ACP has no way to inject into a turn already running, so the
neutral layer's queue emulates it by holding the prompt until `TurnEnded` — and
that difference has to reach the UI, because a composer that says "sent" about a
prompt still sitting in a queue is telling the user something untrue. Both
native backends accept a message into the running turn (the SDK's `streamInput`
for claude, `turn/steer` for codex), so `ChatSession::steer_queued`
(`crates/agent/src/chat.rs`) actually sends it instead.

**Replay.** ACP decides this per connection: an adapter advertises
`agentCapabilities.loadSession` at `initialize` or it does not, and the answer
differs between the four adapters shipped above (`AcpBackend` in
`crates/acp/src/backend.rs`). Both native backends report it unconditionally —
resuming is `--resume` for claude and `thread/resume` for codex, a launch flag
and a method rather than an optional capability.

**File IO** is a difference rather than a win. Under ACP the agent asks Far
Cooler to touch the disk, so every path it names is untrusted until
`fs_guard::confine` has agreed it is inside the worktree; both CLIs do their own
file IO and never ask, so there is nothing for that guard to apply to.

The concrete one, past the flags: **subagent parentage**. ACP has to smuggle it
through `_meta.claudeCode` (`crates/acp/src/wire.rs` — `parentToolUseId`,
`subagent`, `toolResponse`, all inside a vendor extension on a frame that has no
field for them). Natively it is a first-class field on the frame itself, on the
finished message and on the streaming deltas alike: `parent_of` in
`crates/claude/src/normalize.rs`, asserted by
`a_subagents_words_carry_the_dispatch_they_belong_to`.

And there is no package in the supply chain to lose. The native path runs the
agent's own installed binary, so nothing can be renamed or deprecated out from
under it — the same argument the `opencode` row makes above, now available to
the two agents that otherwise each depend on an npm adapter fetched through
`npx`. A cold start has nothing to download, either.

## The version pin is what native costs

An ACP adapter will talk to whatever `claude` or `codex` you have installed. A
native backend will not: these types were generated and confirmed against one
CLI release, and a version outside the pin is `BackendError::Incompatible`,
which names both versions so a reader can tell which side is behind without
running anything else.

| Agent | Pinned to | Where the number lives | Refuses on |
|---|---|---|---|
| `claude` | 2.1.226 | `PINNED_CLAUDE_VERSION`, a literal in `crates/claude/src/handshake.rs` | a different major **or** minor |
| `codex` | 0.147.0 | `vendor/PINNED`, read at compile time by `PINNED_CODEX_VERSION` in `crates/codex/src/handshake.rs` | a different major only |

The asymmetry is deliberate and the codex side is the correction: it compared
major and minor for exactly one afternoon, during which codex went from 0.146.0
to 0.147.0 and chat mode refused to start on a protocol that had not visibly
changed. The comment on its `check_version` calls a guard that fires on an
ordinary release "an outage on a schedule somebody else controls", and leans
instead on the normalizer being lenient — a frame this build does not know
becomes a visible `Gap` (`AgentGapReason::Unparsed`) rather than a broken
session. Claude's guard is looser than an exact pin for the same kind of reason:
the CLI ships patch releases constantly, so 2.1.300 is accepted and 2.2.0 is not
(`a_patch_release_of_the_same_series_is_still_compatible`).

**The two backends do not enforce the pin at the same moment, and today that
gap is real.** `CodexBackend::start` checks the version out of the `initialize`
result before doing anything else (`crates/codex/src/backend.rs`), so a codex
outside the pin refuses when the pane switches to chat. `ClaudeBackend::start`
does not: the CLI's `initialize` response carries no version at all — the
handshake has to send a second `get_binary_version` request for it — so the
number is checked by `farcooler adapter test claude` and by
`the_claude_native_backend_handshakes_against_the_installed_binary`, and a chat
pane on a claude 2.2 would start anyway and degrade frame by frame into `Gap`s
rather than refusing. Test before trusting it.

When a native backend does refuse, the pane stays a terminal and says so rather
than silently becoming ACP — a fallback would hand you a quietly different
transcript, with fewer item types and no steering, for a protocol you did not
choose (`Status::BackendFailed`, `crates/cli/src/agent_host.rs`):

> farcooler: the native backend for `codex` (`codex`) could not start.
> this agent speaks protocol 1.0.0, but this build was generated against 0.147.0
> Set `backend = "acp"` under `[adapters.codex]` in ~/.config/farcooler/config.toml
> to use the ACP adapter instead, then run `farcooler daemon ensure`.
> Terminal mode needs no backend and is unaffected.

**This path is newer than the ACP one, which is the reason it is opt-in rather
than the default.** Nothing ships on it: `every_built_in_adapter_ships_speaking_acp`
in `crates/agent/src/dispatch.rs` asserts every built-in preset's adapter is
`Acp`, so reaching a native backend today means editing the file or flipping the
picker yourself. What is proven is that both start and answer — the two tests in
`crates/agent/tests/backends.rs` complete a real handshake against the installed
`claude` and `codex`, and a missing binary fails those tests rather than
skipping them. One stale string to ignore on the way: the Mac editor's own
footer still reads "Chat mode still runs the ACP adapter — Test proves the
native handshake only", which describes an earlier state — `start_backend` in
`crates/cli/src/agent_host.rs` dispatches on the same field that picker writes
and starts the native backend for the pane.

## How detection works

A pane is identified two ways, in order: first by its running process
(`commands`, matched as a prefix — `codex-aarch64-a` has to match `codex`
because tmux truncates the field), then, if that fails, by scanning the
visible screen for one of its `identity` strings. Screen matching exists
because process matching is not reliable: Claude Code renames its process to
its own version number (`2.1.220`), and `cursor-agent` runs as plain `node`,
which is far too generic to ever claim.

**The rule for writing an `identity` string: it must be furniture the agent
always draws on screen, never a phrase a user could type.** A shell that
happens to echo `"do you want to proceed?"` must not be promoted to an agent
just because the text matches — `activity.rs`'s own test for this,
`a_shell_showing_agent_like_text_is_not_promoted_to_an_agent`, asserts that
`identify("zsh", "$ echo 'do you want to proceed?'")` finds nothing. Every
built-in's `identity` list was read off a running instance of that agent, not
guessed at — the file's own header comment says the first version of this
table was guesswork and matched no real screen.

## Two known gaps

**opencode's `blocked` list is deliberately empty.** Every real attempt to
trigger its permission-approval screen — a file write, a file delete, a
network fetch via `curl`, `sudo -n`, and the same combination repeated in a
directory this opencode install had never touched before, to rule out a
remembered per-directory trust grant — ran with no approval prompt at all.
The one screen this session ever produced that genuinely needed a human was a
provider's own billing-error text bubbling through opencode's footer, not
opencode's own furniture — a user on a different provider, or with balance,
would never see it — so it was left out of `blocked` rather than shipped as a
rule that looks like coverage but fires for almost nobody. `blocked` stays
`Vec::new()`, and that same billing-error screen classifies as `Working`
(its `esc interrupt` hint is still present), not `Blocked` — the honest
result of leaving the list empty rather than a bug to paper over. Until
someone observes opencode's real permission screen, it will not report
"needs you" in the fleet view the way `claude`, `codex`, and `cursor` do. See
the comment on `blocked: Vec::new()` in the opencode entry in
`crates/core/src/activity.rs` for the full account.

**Two of cursor's three `identity` strings remain unverified.** Only
`"Press any key to sign in"` has been confirmed against a real screen — it is
the exact text of cursor-agent's sign-in wall. `"Cursor Agent"` and
`"cursor-agent"` are still in the table but were never seen on that screen or
anywhere past it, because cursor-agent renders its own product banner as
block-graphic ASCII art rather than literal text, and getting past the
sign-in wall means starting an OAuth flow this project has not attempted.
They are left in place — there is no evidence either way, and removing them
on a guess would be its own kind of fabrication — but they should be checked
against a signed-in cursor-agent before anyone trusts them.
