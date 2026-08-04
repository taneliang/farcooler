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

**The config file is read once, at daemon startup, not on every toggle.**
`Service::open_in` (`crates/daemon/src/service.rs`) loads it into a `Registry`
held for the life of the process — deliberately, per the comment on
`Service`'s `registry` field: the file and the environment that locates it are
process-global, and a service that consulted them on every call could be moved
out from under itself by another thread, exactly as `root` could. Editing
`~/.config/farcooler/config.toml` therefore does nothing until the daemon
restarts. Run `farcooler daemon ensure` to pick it up — it replaces a running
daemon with one built from the same source, which for a config-only edit is
just a restart.

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
