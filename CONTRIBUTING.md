# Contributing to Far Cooler

Thanks for helping. This file covers building each part, running the tests and
the checks CI runs, and the conventions a change is reviewed against. For what
the product is, start with the [README](README.md).

## Reporting a bug

Open an issue on GitHub. Say which build you're on (Far Cooler ▸ About Far
Cooler shows it, and `farcooler --version` prints it), which
platform, and what you expected to happen. A runner's state is often the
answer, so include `farcooler status` (or `farcooler runner status you@box`
for a Linux runner) when it's relevant.

## Repository layout

```
crates/
├── protocol     protobuf types, length-delimited framing, wire limits
├── core         resource models, the derivation rule, errors
├── store        SQLite: durable identity and intent only
├── tmux         the private tmux server, control mode, the live runtime inventory
├── transport    Unix socket and SSH stdio adapters, backpressure
├── daemon       farcoolerd: git worktree transactions, the board, domain services
├── cli          the farcooler command
├── vt           the terminal emulator every client renders from, behind a C ABI
├── client       "talk to a runner": SSH, the protocol, and a C ABI over both
├── android      a JNI shim over client and vt, and nothing else
├── agent-core   the event vocabulary every agent backend produces
├── agent        the agent view's runner side, over the three backends below
├── acp          the Agent Client Protocol backend, the extension point for any agent
├── claude       the Claude Code stream-json backend
├── codex        the Codex app-server backend
├── agent-hooks  parsing agents' lifecycle hooks
├── review       diff parsing, and anchoring review comments to a moving diff
├── fence        the fenced block Far Cooler owns in authorized_keys and ssh config
├── ffi-guard    the panic guard every function the apps call goes through
└── tailcat      the tunnel, a Go library in crates/tailcat/go
apps/macos       the Mac app, SwiftUI
apps/ios         the iPhone, Apple Watch, widget and Live Activity targets
apps/android     the Android app, Compose, over the same Rust cores through JNI
apps/shared      AgentKit: logic the Apple apps must agree on, bit for bit
services/relay   the push notification relay, a Cloudflare Worker
proto/           the wire protocol's source of truth
scripts/         builds, releases and the CI checks
docs/            design notes; start with farcooler-design.md
```

The parts that must not differ between clients live in Rust, once, and each
platform writes only a renderer. [`apps/android/README.md`](apps/android/README.md)
explains what that means for the Kotlin side.

## Prerequisites

- **Rust**, stable, 1.85 or later. If `cargo` isn't on your `PATH` (rustup puts
  it in `~/.cargo/bin`), `apps/macos/build-app.sh` fails at its CLI step.
- **tmux 3.x** and **git**, for the daemon and its tests. A few daemon tests
  also want `git-lfs` and `fish`; with `CI` set they fail rather than skip when
  one is missing.
- **Go**, the version in `crates/tailcat/go/go.mod`, for the tunnel. Only the
  Mac app bundle, the phone frameworks and the Linux release binaries need it;
  a plain `cargo build` doesn't.
- **Xcode 27** with the macOS 26 and iOS 26 SDKs, for the Apple apps. The
  watchOS platform is a separate download (`xcodebuild -downloadPlatform
  watchOS`).
- **JDK 17 or later**, the Android SDK with platform 37, and an NDK, for
  Android. Gradle refuses an older JDK, and on macOS `/usr/libexec/java_home -v
  17` can quietly hand back an older one, so set `JAVA_HOME` to the JDK's path
  directly.
- **musl cross-compilers**, only to build Linux runner binaries on a Mac:
  `brew install FiloSottile/musl-cross/musl-cross`.

## Building

### The CLI and the daemon

```sh
cargo build --release
./target/release/farcooler --help
```

`farcoolerd` doesn't parse flags: run with `--help`, it starts a daemon on
your default runtime directory. Point any daemon you built at a scratch one
first, with a short path, since the daemon's Unix socket path must fit in 104
bytes:

```sh
FARCOOLER_HOME=/tmp/fc-scratch ./target/release/farcoolerd
```

A local build is the `local` channel, so it never shares a database, a tmux
server or a binary name with an installed canary or stable app.

### The Mac app

```sh
apps/macos/build-app.sh
open "apps/macos/build/Far Cooler Local.app"
```

`swift build` alone produces a bare executable that can't take keyboard focus;
the script assembles a real bundle, with the CLI and the daemon it ships
inside. It needs Go, for the tunnel the bundled daemon links. A build on your
own machine is the `local` channel, so it's `Far Cooler Local.app`, and it
installs beside a canary or stable app rather than over it.

### iPhone and Apple Watch

```sh
./scripts/build-ios-frameworks.sh --device
apps/ios/generate-project.py
open apps/ios/FarCooler.xcodeproj
```

The project file is generated: add a new Swift file to
`apps/ios/generate-project.py`, not to Xcode. For a simulator build, name a
concrete simulator (`-destination 'platform=iOS Simulator,name=iPhone 17'`):
the frameworks carry arm64 slices only, so a generic destination fails to
link. A channel you've never built on your Apple ID needs its App Group set up
once first; `scripts/portal-app-groups.rb` explains why and how.

To try the app against a throwaway runner without touching your Mac's own,
`./scripts/demo-host.sh` starts a private sshd and daemon for the booted
simulator, and `./scripts/demo-host.sh stop` removes them.

### Android

```sh
./scripts/build-android-libs.sh
cd apps/android && ./gradlew installDebug
```

[`apps/android/README.md`](apps/android/README.md) has the details.

### A Linux runner

```sh
./scripts/build-linux.sh x86_64      # or aarch64
farcooler runner install you@box
```

[`docs/runners.md`](docs/runners.md) covers what `runner install` does and
how to build on the runner itself instead.

## Testing

| What | Command |
| --- | --- |
| Rust | `cargo test --workspace --no-fail-fast` (CI wraps it in `./scripts/tmux-leak-check.py --`, which fails a test that leaves a tmux server running) |
| Rust lints | `cargo clippy --workspace --all-targets -- -D warnings` |
| AgentKit | `swift test --package-path apps/shared/AgentKit` |
| Mac app | `apps/macos/test.sh` (builds the Rust cores the app links, then runs `swift test`; takes `swift test`'s arguments) |
| iOS UI tests | `./scripts/ios-ui-tests.sh` |
| Android | `cd apps/android && ./gradlew testInstrumentedUnitTest` |
| Relay | `cd services/relay && npm ci && npm test` |
| Tunnel | `cd crates/tailcat/go && gofmt -l . && go vet ./... && go test ./...`, once with `CGO_ENABLED=1` and once with `0` |

A few things that save time:

- `cargo test --workspace` stops at the first failing test binary unless you
  pass `--no-fail-fast`, so a run with one failure says nothing about the
  crates after it.
- Run the iOS UI suite through `scripts/ios-ui-tests.sh`, not a hand-written
  `xcodebuild test`. A run in which every test skipped still prints `TEST
  SUCCEEDED`; the script fails it.
- Android's unit-test task is `testInstrumentedUnitTest`. `testDebugUnitTest`
  doesn't exist in this project.
- A test that spawns a daemon needs a short temporary directory on macOS,
  for the same socket-path limit as above: `TMPDIR=/tmp/fc-t cargo test …`.

### Tests that run real agents

Far Cooler recognizes each agent by what it draws on screen, and that changes
with no changelog. A second suite drives the real `claude`, `codex` and
`cursor-agent` to check the rules still hold:

```sh
FARCOOLER_LIVE_AGENTS=1 cargo test -p farcooler-core --test live_agents -- --ignored --nocapture
```

Every test that starts a real agent is `#[ignore]` and also needs
`FARCOOLER_LIVE_AGENTS=1`, so neither `cargo test` nor `cargo test --
--ignored` starts one by accident. With the switch on, they need the CLIs
installed and signed in, and they cost real tokens. Run them after touching
`crates/core/src/activity.rs` or `title.rs`. A failure writes the captured
screen to `target/live-agents/`; once the rules are fixed it belongs in
`crates/core/captures/`. [`crates/core/tests/live_agents.rs`](crates/core/tests/live_agents.rs)
has the rest, and [`test/live_agents.rs`](test/live_agents.rs) is the switch
every such test asks first.

## The checks CI runs

Each has a `--self-test` that checks the check itself. Run them before you
push:

| Script | What it refuses |
| --- | --- |
| `./scripts/file-size-budget.py` | A source file growing past 1,500 lines. Files already over it are listed in `scripts/file-size-budget.txt` and may only shrink, within a small margin. |
| `./scripts/doc-comment-check.py --base origin/main --head HEAD` | A doc comment left sitting on the wrong item after an insertion. |
| `./scripts/swallow-lint.py` | A client call whose failure is silently dropped. Justified exceptions live in `scripts/swallow-lint-allow.txt`. |
| `./scripts/proto-lint.py` | A wire change that would break a client already in the field. See [`docs/releasing.md`](docs/releasing.md#the-wire-has-the-same-rule). |
| `./scripts/copy-lint.py` | A count in parentheses in any app's copy. |
| `./scripts/visual-tokens-lint.py` | A hand-drawn radius, rule, shadow, material or fill in a Mac or AgentKit view, where a design token exists. |

`./scripts/install-git-hooks.sh` installs a `commit-msg` hook that runs the
doc comment check on every commit. A doc comment you moved on purpose takes a
`Doc-Comment-Check: moved` trailer in the commit message.

## Conventions

- **Never run `cargo fmt`.** The Rust tree is formatted by hand, and default
  rustfmt would rewrite much of it; CI skips `fmt --check` on purpose. Match the
  style around your change: about 100 columns, short struct literals on one
  line. Go code under `crates/tailcat/go` is the exception, and is `gofmt`'s.
- **Doc comments say why.** Most items carry a comment explaining the decision
  behind them and the failure it prevents. Keep that comment with its item when
  you insert code nearby, and update it when you change what it describes.
- **Tests must be able to fail.** Show a new test failing without your fix, or
  with the fix reverted at its call site. A test that passes on the old code
  isn't testing the change.
- **Copy follows Apple's conventions, in US English.** Buttons and menu items
  in title case, an ellipsis on anything that opens more UI, contractions, no
  counts in parentheses, and never a raw Rust or SSH error on screen. Color is
  for states that need the person's attention.
- **Sign your commits**, and write the subject as `area: what changed`, as
  `git log` shows.
- **No merge commits on `main`.** Pull requests land by squash.
- **Never hand-edit generated files**: `apps/ios/FarCooler.xcodeproj` comes
  from `apps/ios/generate-project.py`, and the protocol types from `proto/`.

## Further reading

- [`docs/farcooler-design.md`](docs/farcooler-design.md): the design, and the
  rule that runtime state is derived from tmux and never stored.
- [`docs/workspaces.md`](docs/workspaces.md): workspaces, the orchestrator and
  the board, as every surface draws them.
- [`docs/runners.md`](docs/runners.md): runners, SSH and the tunnel.
- [`docs/releasing.md`](docs/releasing.md): versions, channels, CI and
  updates.
- [`docs/adapters.md`](docs/adapters.md): chat mode and ACP adapters.
- [`docs/agent-session-logs.md`](docs/agent-session-logs.md): what each agent
  writes to disk.
- [`TODOS.md`](TODOS.md): deferred work.
