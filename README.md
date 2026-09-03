# Far Cooler

A terminal-first command center for parallel coding agents on runners you own.

A **workspace** is one git worktree plus one branch for one task, along with its
terminals and agent processes. Far Cooler lets you run several at once and see,
truthfully, which are alive. Both words are real, and each has a job — see
[`docs/workspaces.md`](docs/workspaces.md).

A **runner** is one `farcoolerd`: one Unix user, on one host, with its own
worktrees and its own `~/.ssh/authorized_keys`. A host may carry several — three
engineers sharing a Linux box is three runners, sharing nothing — which is why
the word is not *host*. See [`docs/runners.md`](docs/runners.md).

Design: [`docs/farcooler-design.md`](docs/farcooler-design.md).
Deferred work: [`TODOS.md`](TODOS.md).

## The one idea worth knowing

**Runtime state is derived, never stored.**

SQLite holds only what must outlive tmux: which workspaces exist, which branch
each is on, and what you *intended* each terminal to be doing. tmux is the sole
authority on whether a process is alive right now.

The same rule decides what a workspace is called. Its name is its worktree's
directory read back as prose, never a stored title — so there is no name that
can disagree with the directory it describes, and none to keep in sync. Not the
branch: one worktree hosts a stack of commits over its life, so naming it after
the branch inside it would rename it every time the work moved forward.

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

Go 1.23+ is needed only to build the tunnel archive
(`./scripts/build-tailcat.sh <target>`), which the platform build scripts link
into shipping builds so they can reach runners behind NAT. `cargo build`
itself never needs Go: it always produces a working `farcooler` that reaches
runners by address, and reports `no_tailcat` for tunneled ones.

The macOS app:

```sh
cd apps/macos && swift build
```

## Tests

```sh
cargo test --workspace
```

That never runs the agents. Far Cooler recognizes claude, codex and
cursor-agent by furniture they draw on screen, and that furniture changes with
no changelog — so there is a second suite that drives the real binaries and
checks the rules still hold:

```sh
cargo test -p farcooler-core --test live_agents -- --ignored --nocapture
```

Run it after touching `activity.rs` or `title.rs`, and periodically to catch a
third-party release. It costs a few cents and a few minutes, needs the CLIs
already signed in, and SKIPS rather than fails when one is missing. A check that
fails writes the captured screen to `target/live-agents/` — that file is both
the bug report and the fix, since it belongs in `crates/core/captures/` once the
rules are corrected.

## Quick start

```sh
# 1. Allowlist a directory Far Cooler may operate in.
farcooler root add ~/Dev

# 2. Register a repository inside it.
farcooler repo register ~/Dev/my-project

# 3. Create task workspaces. Each is a real git worktree on a new branch.
farcooler repo list                       # note the id
# The name becomes the worktree's directory, and cannot be changed later.
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

A pane running one of the recognized agents can be flipped from a terminal
into a native chat (`⌃B a` in the Mac app). That needs an ACP adapter, which
not every agent has — see [`docs/adapters.md`](docs/adapters.md) for which
agents ship one, the config file for adding your own, and known gaps.

## The phone apps

```sh
# iOS — needs Go (see Build, above): --device builds and links the real
# tunnel archive into farcooler-client, and fails outright without one
# rather than quietly shipping a build with no tunnel.
./scripts/build-ios-frameworks.sh --device && open apps/ios/FarCooler.xcodeproj

# Android — stays on the tunnel stub regardless: Go's own toolchain refuses
# `-buildmode=c-archive` for android/arm64.
./scripts/build-android-libs.sh && (cd apps/android && ./gradlew installDebug)
```

The iOS app and its four extensions share an App Group, and Apple's own API
cannot attach one to an App ID — so a channel you have never built on this
Apple ID needs one manual pass first:

```sh
FASTLANE_USER=you@example.com ruby scripts/portal-app-groups.rb local
```

Skip it and the build still compiles and signs; it fails at INSTALL, on the
device, with *"This app cannot be installed because its integrity could not be
verified"* — which reads like a broken certificate and is not one. See the
script's own header for why it needs a 2FA login when nothing else here does.

Both connect over SSH with a key the device generates and never hands out, and
both render from the same Rust cores the Mac does. The Android client is the
newer of the two and connects to every configured runner at once, the way the
Mac does; the iOS client still switches between them.
[`apps/android/README.md`](apps/android/README.md) has the full list of what
differs and why.

## The macOS app

```sh
FARCOOLER_BIN=$PWD/target/release/farcooler ./apps/macos/.build/debug/Far Cooler
```

A fleet sidebar, per-terminal output, and an input box. It renders the state the
daemon derived and never computes state itself, so two clients cannot disagree
about the same terminal.

Every runner added under Settings ▸ Runners is connected at once, over SSH,
alongside this Mac — there is no picker and no "current runner" to switch
between first. Projects from every runner appear in one sidebar, each naming
the runner it is on. A runner that stops answering keeps its rows, dimmed,
rather than dropping them: reads keep showing the last good fetch, and an
action against that runner is refused at once with its own error instead of
hanging or being queued for later.

The app owns this Mac's own daemon specifically — the one runner it can start
and stop directly, because it ships one inside its own bundle. It runs
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
├── cli         the farcooler command
├── vt          the terminal emulator every client renders from
├── client      "talk to a runner": ssh, protocol, and a C ABI over both
└── android     a JNI shim over `client` and `vt`, and nothing else
apps/macos      SwiftUI client
apps/ios        SwiftUI client, over `client`'s C ABI
apps/android    Compose client, over the same ABI through JNI
apps/shared     the logic the two Apple apps must agree on, bit for bit
proto/          canonical protocol source of truth
```

`core` defines the `RuntimeInventory` trait and `tmux` implements it, so crate
dependencies point one way and the derivation rule is unit-testable with no tmux
running.

`vt` and `client` exist for the same reason as each other: the parts that must
not differ between clients live in Rust, once, and each platform writes only a
renderer. `apps/android`'s Kotlin never parses an escape sequence, never decides
what an arrow key sends, and never speaks the protocol — see
[`apps/android/README.md`](apps/android/README.md).

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

Also working: every configured runner connects over SSH at once, alongside
this Mac, with its own reconnection and backoff — see `docs/farcooler-design.md`
for the connection states this is built on.

Also working: iOS and Android clients, over one Rust implementation of the SSH
transport, the protocol and the terminal emulator — a C ABI that Swift imports
directly and Kotlin reaches through the JNI shim in `crates/android`.

Not built yet: the daemon's socket server is not wired to the CLI (the CLI links
the service directly), and the terminal channel streams via `capture-pane`
rather than control-mode streaming. See the final section of the design doc for
the full picture.

## License

Copyright © 2026 E-Liang Tan. Far Cooler is licensed under the
[MIT License](LICENSE).

The bundled Iosevka Nerd Font Mono files remain licensed under the SIL Open Font
License 1.1; see
[`apps/ios/FarCooler/Fonts/IOSEVKA-LICENSE.md`](apps/ios/FarCooler/Fonts/IOSEVKA-LICENSE.md).
The Android app ships the same files under the same licence.
