# Recognizing agents

Every pane in Far Cooler is a terminal: a shell, or a coding agent's own TUI
running inside one, exactly as if you had typed the command yourself. For
claude and codex, the apps draw a conversation view over that TUI — its rows
read from the agent's own transcript (`agent.rows`), its composer typing into
the agent's box (`terminal.compose`) — and the terminal is always one tap
away.

Far Cooler used to offer a second way to host an agent: **chat mode**, where
`⌃B a` swapped the pane's TUI for a native chat driven over the Agent Client
Protocol, Claude Code's `stream-json` or Codex's `app-server`. It was retired
(ov-377), because the conversation view over the real TUI does the same job
without a second copy of each agent's protocol to keep in step. A pane still
in chat mode when its runner updates moves to its TUI at the agent's next idle
— `claude --resume` or `codex resume` on the same conversation — with a line in
the pane saying why (`crates/daemon/src/chat_migration.rs`). `⌃B a` now toggles
the conversation view.

What is left of the adapter table is detection: how the daemon knows which
agent a pane is running, and whether it is working, blocked or idle.

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
cost every other agent its recognition, so `tracing::warn!` names the file and
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
Editing an `[adapters.*]` table therefore does nothing until the daemon
restarts. Run `farcooler daemon ensure` to pick it up — it replaces a running daemon with one
built from the same source, which for a config-only edit is just a restart.

The other two tables are read per call, so editing them takes effect on the next
one. Themes are read that way because a color is something you tune by looking
at it; branches because the runner-settings editor writes that table, and a
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
the CLI agree with the runner the worktree is created on rather than each other.

## An `[adapters.<name>]` table

The name is chat mode's, kept so a file written for it still works:

```toml
[adapters.my-agent]
identity = ["My Agent v"]
commands = ["my-agent"]
blocked = ["Allow this command?"]
working = ["esc to interrupt"]
```

Naming an existing built-in preset (`claude`, `codex`, `opencode`, `cursor`)
changes its detection; naming anything else adds an agent. A new one must
supply `identity` or `commands`, or nothing can ever identify a pane as it.
Only the fields a table actually supplies are overwritten, because TOML has no
way to say "leave this field alone". The launch fields chat mode read
(`program`, `args`, `env`, `backend`) are read past rather than refused, so an
old file still parses.

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
