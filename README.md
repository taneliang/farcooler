# Far Cooler

A terminal-first command center for parallel coding agents on hosts you own.

A **workspace** is one git worktree plus one branch for one task, along with its
terminals and agent processes. Far Cooler lets you run several at once and see,
truthfully, which are alive.

Design: [`docs/farcooler-design.md`](docs/farcooler-design.md).
Deferred work: [`TODOS.md`](TODOS.md).

## The one idea worth knowing

**Runtime state is derived, never stored.**

SQLite holds only what must outlive tmux: which workspaces exist, which branch
each is on, and what you *intended* each terminal to be doing. tmux is the sole
authority on whether a process is alive right now.

There is no database column in which a stale `running` could ever be written, so
Far Cooler structurally cannot tell you an agent is running after it died. When a
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
./target/release/farcooler --help
```

The macOS app:

```sh
cd apps/macos && swift build
```

## Quick start

```sh
# 1. Allowlist a directory Far Cooler may operate in.
farcooler root add ~/Dev

# 2. Register a repository inside it.
farcooler repo register ~/Dev/my-project

# 3. Create task workspaces. Each is a real git worktree on a new branch.
farcooler repo list                       # note the id
farcooler workspace create <repo-id> "add auth"   --branch feat/auth
farcooler workspace create <repo-id> "fix parser" --branch fix/parser

# 4. Launch a terminal in each.
farcooler workspace list                  # note the workspace ids
farcooler terminal create <ws-id> --preset claude
farcooler terminal create <ws-id> --preset shell

# 5. See the fleet, with every state derived fresh from tmux.
farcooler workspace list
```

```
b8aed78c  add auth                active    feat/auth
    bb5a5df9  claude            running   claude
f0a16d76  fix parser              active    fix/parser
    ba941f48  shell             running   shell
```

Drive a terminal:

```sh
farcooler terminal send <term-id> 'git status
'
farcooler terminal read <term-id> --lines 50
```

Recover one that died:

```sh
farcooler terminal restart <term-id>       # new epoch, same preset
farcooler terminal dismiss-lost <term-id>  # forget it, without claiming an exit
```

Attach to the real tmux session:

```sh
farcooler attach <ws-id>     # prints the exact command
```

## Presets

`shell`, `claude`, `codex`, `cursor`, or any command. Presets run through your
configured shell as an interactive login shell, so version managers, `direnv`,
aliases, and startup files behave exactly as in a hand-launched terminal.

## The macOS app

```sh
FARCOOLER_BIN=$PWD/target/release/farcooler ./apps/macos/.build/debug/Far Cooler
```

A fleet sidebar, per-terminal output, and an input box. It renders the state the
daemon derived and never computes state itself, so two clients cannot disagree
about the same terminal.

The app owns this Mac's daemon. It ships one inside its own bundle and runs
`farcooler daemon ensure` at launch, which replaces any daemon built from
different source than the app:

```sh
farcooler daemon ensure   # start one, or replace a mismatched one
farcooler daemon stop     # terminals keep running: they belong to tmux
```

Two components built from different source speak the same protocol perfectly and
still behave like two different programs, and the symptom is a bug you already
fixed still happening.

## Layout

```
crates/
├── protocol    protobuf types, length-delimited framing, wire limits
├── core        resource models, the derivation rule, errors, replay buffer
├── store       SQLite: durable identity and intent only
├── tmux        private tmux server, control mode, the live runtime inventory
├── transport   Unix socket and stdio adapters, backpressure
├── daemon      git worktree transactions, domain services
└── cli         the farcooler command
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
- Hiding a workspace never touches git, and is never refused for a running
  terminal — it is a view preference, not a lifecycle step. Removing a worktree
  is the one that is refused while a managed terminal is running.
- Git is the source of truth for which worktrees exist. Registering a
  repository adopts every worktree it already has, and a reconcile pass keeps
  the sidebar honest against `git worktree add`/`remove` run outside Far Cooler.
- Identity comes only from exact tmux tags. Names, indexes, and PIDs are
  diagnostic and never establish identity, so Far Cooler never adopts a process it
  did not launch.

## Status

Working today: the Mac-first local slice. Repository roots, repositories,
workspaces with real git worktrees, terminals in a private tmux server, derived
fleet state, input and output, restart, loss dismissal, hide/unhide, and the
SwiftUI app.

Not built yet: the daemon's socket server is not wired to the CLI (the CLI links
the service directly), the terminal channel streams via `capture-pane` rather
than control-mode streaming, and there is no SSH transport, iOS client, or
libghostty terminal core. See the final section of the design doc for the full
picture.

## License

Copyright © 2026 E-Liang Tan. Far Cooler is licensed under the
[MIT License](LICENSE).

The bundled Iosevka Nerd Font Mono files remain licensed under the SIL Open Font
License 1.1; see
[`apps/ios/FarCooler/Fonts/IOSEVKA-LICENSE.md`](apps/ios/FarCooler/Fonts/IOSEVKA-LICENSE.md).
