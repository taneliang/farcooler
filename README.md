# Overnight

A terminal-first command center for parallel coding agents on hosts you own.

A **workspace** is one git worktree plus one branch for one task, along with its
terminals and agent processes. Overnight lets you run several at once and see,
truthfully, which are alive.

Design: [`docs/overnight-design.md`](docs/overnight-design.md).
Deferred work: [`TODOS.md`](TODOS.md).

## The one idea worth knowing

**Runtime state is derived, never stored.**

SQLite holds only what must outlive tmux: which workspaces exist, which branch
each is on, and what you *intended* each terminal to be doing. tmux is the sole
authority on whether a process is alive right now.

There is no database column in which a stale `running` could ever be written, so
Overnight structurally cannot tell you an agent is running after it died. When a
terminal is expected to be alive and no live exactly-tagged pane proves it, the
answer is `LOST` rather than a guess.

## Requirements

- macOS (Apple silicon or Intel), or Linux
- `tmux` 3.x
- `git`
- Rust 1.85+ to build

## Build

```sh
cargo build --release
./target/release/overnight --help
```

The macOS app:

```sh
cd apps/macos && swift build
```

## Quick start

```sh
# 1. Allowlist a directory Overnight may operate in.
overnight root add ~/Dev

# 2. Register a repository inside it.
overnight repo register ~/Dev/my-project

# 3. Create task workspaces. Each is a real git worktree on a new branch.
overnight repo list                       # note the id
overnight workspace create <repo-id> "add auth"   --branch feat/auth
overnight workspace create <repo-id> "fix parser" --branch fix/parser

# 4. Launch a terminal in each.
overnight workspace list                  # note the workspace ids
overnight terminal create <ws-id> --preset claude
overnight terminal create <ws-id> --preset shell

# 5. See the fleet, with every state derived fresh from tmux.
overnight workspace list
```

```
b8aed78c  add auth                active    feat/auth
    bb5a5df9  claude            running   claude
f0a16d76  fix parser              active    fix/parser
    ba941f48  shell             running   shell
```

Drive a terminal:

```sh
overnight terminal send <term-id> 'git status
'
overnight terminal read <term-id> --lines 50
```

Recover one that died:

```sh
overnight terminal restart <term-id>       # new epoch, same preset
overnight terminal dismiss-lost <term-id>  # acknowledge without claiming an exit
```

Attach to the real tmux session:

```sh
overnight attach <ws-id>     # prints the exact command
```

## Presets

`shell`, `claude`, `codex`, `cursor`, or any command. Presets run through your
configured shell as an interactive login shell, so version managers, `direnv`,
aliases, and startup files behave exactly as in a hand-launched terminal.

## The macOS app

```sh
OVERNIGHT_BIN=$PWD/target/release/overnight ./apps/macos/.build/debug/Overnight
```

A fleet sidebar, per-terminal output, and an input box. It renders the state the
daemon derived and never computes state itself, so two clients cannot disagree
about the same terminal.

## Layout

```
crates/
├── protocol    protobuf types, length-delimited framing, wire limits
├── core        resource models, the derivation rule, errors, replay buffer
├── store       SQLite: durable identity and intent only
├── tmux        private tmux server, control mode, the live runtime inventory
├── transport   Unix socket and stdio adapters, backpressure
├── daemon      git worktree transactions, domain services
└── cli         the overnight command
apps/macos      SwiftUI client
proto/          canonical protocol source of truth
```

`core` defines the `RuntimeInventory` trait and `tmux` implements it, so crate
dependencies point one way and the derivation rule is unit-testable with no tmux
running.

## Safety properties

- Repository roots are allowlisted. `/`, system directories, your home directory
  itself, and any path nesting inside an existing root are refused.
- Workspace creation never silently reuses an existing branch or worktree path.
- A failed metadata write rolls back only a provably clean, untouched worktree.
  A dirty one is preserved with the artifacts left in place.
- Archiving is refused while a managed terminal is running, and never touches git.
- Identity comes only from exact tmux tags. Names, indexes, and PIDs are
  diagnostic and never establish identity, so Overnight never adopts a process it
  did not launch.

## Status

Working today: the Mac-first local slice. Repository roots, repositories,
workspaces with real git worktrees, terminals in a private tmux server, derived
fleet state, input and output, restart, loss dismissal, archive, and the SwiftUI
app.

Not built yet: the daemon's socket server is not wired to the CLI (the CLI links
the service directly), the terminal channel streams via `capture-pane` rather
than control-mode streaming, and there is no SSH transport, iOS client, or
libghostty terminal core. See the final section of the design doc for the full
picture.
