# Every Machine In One Fleet — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hold a connection to every configured machine at once, merge their projects into one scroll area with the machine named alongside each, and survive SSH connections dying silently.

**Architecture:** `DaemonClient` becomes per-machine; a new `FleetStore` owns one per configured host plus the local one, merges their fleets, and routes every mutation by `workspace.host`. Resilience starts below the app, in the `ssh` invocation the CLI makes.

**Tech Stack:** Swift/SwiftUI (macOS), Rust (CLI), SSH.

Spec: `docs/superpowers/specs/2026-08-03-every-machine-in-one-fleet-design.md`

## Global Constraints

- **`workspace.host` is the routing key**, empty string meaning this Mac. This is the existing convention (`OpenInEditor.swift:19`); do not invent a second one.
- **The Mac app is a THIN CLIENT.** It renders daemon-derived state and never computes worktree existence or terminal state itself.
- **Every field added to a `Decodable` after the first release is OPTIONAL, not defaulted.** Swift's synthesized `Decodable` throws on a missing key, so a non-optional new field makes a client meeting an older daemon fail to decode the entire fleet and show "no workspaces" for a host full of them.
- **A mutation routed to an unreachable machine is refused by `FleetStore`, not by the call site.** There are ~122 call sites; a rule each must remember is a rule that gets forgotten.
- **Nothing is queued for an offline machine.** An action replayed minutes later against changed state is a surprise nobody can attribute.
- `cargo test --workspace` and `./apps/macos/build-app.sh` must both pass. **`cargo` is at `~/.cargo/bin/cargo` and is NOT on PATH** — `export PATH="$HOME/.cargo/bin:$PATH"` first, or `build-app.sh` fails at its Rust step with `cargo: command not found`.
- Swift has no test infrastructure in this repo. Swift tasks are verified by the build plus the task's own manual checklist. **Do not invent a Swift test target.**
- Commit after every task, using the message given.

---

### Task 1: SSH stops hanging on a dead peer

**Files:**
- Modify: `crates/cli/src/remote.rs:37-57` (`ssh_args`) and its `mod tests`

**Interfaces:**
- Consumes: nothing.
- Produces: no API change. `ssh_args` gains three `-o` options; every caller already routes through it.

- [ ] **Step 1: Write the failing test**

In `crates/cli/src/remote.rs` `mod tests`:

```rust
    /// The options that turn a silent death into an event.
    ///
    /// Without these, a peer that goes away without closing the socket — a lid
    /// that closes, a VPN that drops, a server that reboots — leaves ssh parked
    /// on a half-open TCP connection indefinitely. The process never exits, so
    /// nothing above it ever learns, and the machine keeps looking connected
    /// while delivering nothing. There is no signal for a client to react to,
    /// which is why this cannot be fixed any higher up.
    ///
    /// Asserted by name because their absence is invisible until a connection
    /// dies, which is exactly when nobody is reading test output.
    #[test]
    fn ssh_notices_a_peer_that_stopped_answering() {
        let args = ssh_args("box");
        let has = |k: &str| args.windows(2).any(|w| w[0] == "-o" && w[1].starts_with(k));
        assert!(has("ServerAliveInterval="), "no liveness probe: {args:?}");
        assert!(has("ServerAliveCountMax="), "no probe limit: {args:?}");
        assert!(has("ConnectTimeout="), "a stale control socket could hang: {args:?}");
    }
```

- [ ] **Step 2: Run it and watch it fail**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test -p farcooler-cli ssh_notices_a_peer_that_stopped_answering
```

Expected: FAIL on the first assertion.

- [ ] **Step 3: Add the options**

Replace `ssh_args`'s body, keeping the existing options and doc comment and appending to it:

```rust
/// Options that apply to every ssh invocation.
///
/// `BatchMode=yes` matters: without it a host whose key has changed, or which
/// wants a password, hangs waiting for input that a GUI client will never
/// provide. Failing immediately with ssh's own message is far better than a
/// command that never returns.
///
/// The keepalives matter for the same reason in slower motion. A peer that
/// vanishes without closing its socket leaves ssh waiting forever on a
/// connection that will never carry another byte, and a client watching for the
/// process to exit waits just as long. `ServerAliveInterval` probes; three
/// unanswered probes end the session, so a dead machine is noticed in about
/// forty-five seconds instead of never.
///
/// Fifteen seconds where `crates/client/src/ssh.rs` uses thirty: that path is
/// the phone, which pays for every probe in radio wake-ups. A Mac on mains
/// power does not, and halving the detection time matters for a tool whose
/// purpose is noticing when an agent needs you.
///
/// `ConnectTimeout` bounds the one failure `ControlMaster` can introduce: a
/// stale control socket left by a master that died badly, which a new
/// connection would otherwise wait on indefinitely.
fn ssh_args(target: &str) -> Vec<String> {
    vec![
        "-o".into(),
        "BatchMode=yes".into(),
        // Multiplex over one connection so a burst of commands does not mean a
        // burst of TCP handshakes and key exchanges.
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        "ControlPath=~/.ssh/farcooler-%r@%h:%p".into(),
        "-o".into(),
        "ControlPersist=120".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=3".into(),
        "-o".into(),
        "ConnectTimeout=10".into(),
        target.to_string(),
    ]
}
```

- [ ] **Step 4: Run the tests**

```bash
cargo test -p farcooler-cli
```

Expected: PASS, including the pre-existing `ssh_never_waits_for_input_it_will_not_get`.

- [ ] **Step 5: Commit**

```bash
git add crates/cli/src/remote.rs
git commit -m "fix(cli): let ssh notice a peer that stopped answering

Every ssh invocation set BatchMode and connection multiplexing and no
keepalives at all, so a peer that vanished without closing its socket left
ssh parked on a half-open connection indefinitely. The process never
exited, so nothing above it learned, and the machine kept looking
connected while delivering nothing.

crates/client/src/ssh.rs has had keepalives since it was written -- its
comment starts \"A phone sleeps and wakes\". The lesson was in the repo; it
had just never reached the path the Mac uses.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `DaemonClient` becomes per-machine

**Files:**
- Modify: `apps/macos/Sources/FarCooler/DaemonClient.swift:12-42, 100-102`
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift:6`

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `DaemonClient.init(target: String = "")` — the machine this client drives; empty means this Mac.
  - `DaemonClient.target: String` — read-only.
  - `cliHostArguments` derives from `target`, not from `Preferences.remoteHost`.
  - `hostChanged()` is DELETED.

- [ ] **Step 1: Make the target an instance property**

In `DaemonClient.swift`, add above `cliPath`:

```swift
    /// The machine this client drives. Empty means the Mac it runs on.
    ///
    /// An instance property, not a global preference. One client per machine is
    /// what lets an unreachable one be a single object in a bad state rather
    /// than a condition threaded through shared code — and it is what makes
    /// holding several at once possible at all.
    let target: String

    init(target: String = "") {
        self.target = target.trimmingCharacters(in: .whitespaces)
    }
```

Replace `cliHostArguments`:

```swift
    /// What to put in front of every CLI invocation to aim it at this client's
    /// machine. Empty when it is the machine the app runs on.
    ///
    /// Before the subcommand, not after: `--host` is a top-level option, and
    /// clap will not see it once a subcommand has been named.
    var cliHostArguments: [String] {
        target.isEmpty ? [] : ["--host", target]
    }
```

- [ ] **Step 2: Delete `hostChanged()`**

Remove the whole method (`DaemonClient.swift:89-102`). It exists to re-point one client at a different machine, which is the model being replaced. Remove its call site in `ContentView.swift` — an `.onChange(of: preferences.remoteHost)` around line 97.

- [ ] **Step 3: Keep the app working, unchanged in behaviour**

In `ContentView.swift:6`:

```swift
    @StateObject private var client = DaemonClient(target: Preferences.shared.remoteHost)
```

This is deliberately temporary and preserves today's behaviour exactly: one client, aimed at whatever the picker last selected. Task 5 replaces it with `FleetStore`. Leaving the app in a working state at every commit is what makes the intermediate commits reviewable.

- [ ] **Step 4: Build and check**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
./apps/macos/build-app.sh
```

Expected: builds. Launch it; the sidebar behaves exactly as before, and switching machines in the picker now does nothing until relaunch. That regression is expected and lives for three commits.

- [ ] **Step 5: Commit**

```bash
git add apps/macos/Sources/FarCooler/DaemonClient.swift apps/macos/Sources/FarCooler/ContentView.swift
git commit -m "refactor(macos): a DaemonClient drives one machine

The target moves from a global preference onto the instance. One client
per machine is what lets an unreachable one be a single object in a bad
state rather than a condition threaded through shared code.

hostChanged() goes with it: re-pointing one client at another machine is
the model being replaced. The picker stops taking effect until relaunch
for the next few commits, which is the honest cost of not doing this in
one unreviewable change.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Connection state and reconnection

**Files:**
- Modify: `apps/macos/Sources/FarCooler/DaemonClient.swift` (state, backoff, `startEvents`/`onEnd`)
- Create: `apps/macos/Sources/FarCooler/Reachability.swift`

**Interfaces:**
- Consumes: `DaemonClient.target` (Task 2).
- Produces:
  - `enum HostState: Equatable { case connecting, connected, reconnecting(attempt: Int), unreachable(reason: String), notInstalled }`
  - `DaemonClient.state: HostState` (`@Published`, private setter)
  - `DaemonClient.reconnectNow()` — resets backoff and retries immediately
  - `Reachability.shared.onShouldRetry: (() -> Void)?` — fires on wake and on network regain

- [ ] **Step 1: Add the state**

In `DaemonClient.swift`:

```swift
/// Where a machine's connection stands.
///
/// `notInstalled` is separate from `unreachable` because it is not a failure to
/// retry. It is a machine that needs `host install`, and retrying it forever
/// produces noise instead of the one sentence that would fix it.
enum HostState: Equatable {
    case connecting
    case connected
    case reconnecting(attempt: Int)
    case unreachable(reason: String)
    case notInstalled

    var isUsable: Bool { self == .connected }

    /// What to tell someone whose action was refused.
    var refusal: String? {
        switch self {
        case .connected: return nil
        case .connecting: return "still connecting to this machine"
        case .reconnecting: return "reconnecting to this machine"
        case .unreachable(let why): return why
        case .notInstalled: return "Far Cooler is not installed on this machine"
        }
    }
}
```

And on the client:

```swift
    @Published private(set) var state: HostState = .connecting

    /// How long to wait before the next attempt, in seconds.
    ///
    /// Doubling from 1 to a 30s ceiling, with jitter: several machines
    /// recovering from one network event must not retry in lockstep, or the
    /// first thing a just-returned network sees is a thundering herd.
    private var attempt = 0
    private var retryTask: Task<Void, Never>?

    private var backoffSeconds: Double {
        let base = min(30.0, pow(2.0, Double(attempt)))
        let jitter = Double.random(in: 0.8...1.2)
        return base * jitter
    }
```

- [ ] **Step 2: Drive it from the event stream's lifetime**

`startEvents`'s `onEnd` closure currently sleeps two seconds and restarts. Replace that fixed sleep with the backoff, and set state around it:

```swift
            onEnd: { [weak self] in
                Task { @MainActor in
                    guard let self else { return }
                    self.eventStream = nil
                    self.attempt += 1
                    self.state = .reconnecting(attempt: self.attempt)

                    self.retryTask?.cancel()
                    self.retryTask = Task { @MainActor [weak self] in
                        guard let self else { return }
                        try? await Task.sleep(for: .seconds(self.backoffSeconds))
                        guard !Task.isCancelled else { return }
                        // A daemon going away is also the moment another build
                        // could take the socket, so the app claims it back
                        // before reading anything through it.
                        if self.target.isEmpty { await LocalDaemon.shared.ensure() }
                        // Anything that changed while we were deaf is only
                        // visible in a full read.
                        await self.refresh()
                        self.startEvents()
                    }
                }
            })
```

On a successful `refresh()`, reset:

```swift
            attempt = 0
            state = .connected
```

Put that beside the existing `hasLoaded = true; lastError = nil` in `refresh()`'s success path. On the failure path, set `state = .unreachable(reason:)` with the CLI's own message — the daemon's words name what to fix where anything written here would be a guess.

- [ ] **Step 3: Write `Reachability.swift`**

```swift
import AppKit
import Network

/// The two moments when waiting out a backoff is the wrong thing to do.
///
/// A laptop lid closing and opening is the single most common way this feature
/// will be experienced, and without this it means sitting through a thirty
/// second wait while everything looks broken. A network that just came back is
/// the same story with a different cause.
///
/// Deliberately one callback rather than a notification per client: the clients
/// do not each need to know why, only that now is a better moment than the one
/// their timer picked.
@MainActor
final class Reachability {
    static let shared = Reachability()

    /// Called on wake, and when the path goes from unsatisfied to satisfied.
    var onShouldRetry: (() -> Void)?

    private let monitor = NWPathMonitor()
    private var wasSatisfied = true

    private init() {
        NSWorkspace.shared.notificationCenter.addObserver(
            forName: NSWorkspace.didWakeNotification,
            object: nil,
            queue: .main
        ) { [weak self] _ in
            MainActor.assumeIsolated { self?.onShouldRetry?() }
        }

        monitor.pathUpdateHandler = { [weak self] path in
            let satisfied = path.status == .satisfied
            Task { @MainActor in
                guard let self else { return }
                defer { self.wasSatisfied = satisfied }
                // Only the transition INTO reachable. A path that was already
                // satisfied and stayed that way is not news.
                guard satisfied, !self.wasSatisfied else { return }
                self.onShouldRetry?()
            }
        }
        monitor.start(queue: .main)
    }
}
```

- [ ] **Step 4: Add `reconnectNow()`**

```swift
    /// Retry this machine at once, whatever the backoff had planned.
    ///
    /// The escape hatch for the case the timer cannot know about: you fixed the
    /// VPN, and waiting out a thirty second ceiling to find out is the wrong
    /// experience.
    func reconnectNow() {
        retryTask?.cancel()
        retryTask = nil
        attempt = 0
        stopEvents()
        state = .connecting
        Task { @MainActor in
            if target.isEmpty { await LocalDaemon.shared.ensure() }
            await refresh()
            startEvents()
        }
    }
```

- [ ] **Step 5: Build and exercise it by hand**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
./apps/macos/build-app.sh
```

Then, with the app running against the local machine:

| Do this | Expect |
|---|---|
| `farcooler daemon stop` | state leaves `connected`; reconnects on its own within ~2s |
| Stop it three times in a row | the wait grows — 1s, 2s, 4s — rather than staying flat |
| Sleep the Mac and wake it | reconnects at once, not after a wait |

Report what you actually observed, including the timings.

- [ ] **Step 6: Commit**

```bash
git add apps/macos/Sources/FarCooler/DaemonClient.swift apps/macos/Sources/FarCooler/Reachability.swift
git commit -m "feat(macos): a machine's connection has a state, and comes back

Replaces a fixed two second retry with backoff to a thirty second
ceiling, jittered so several machines recovering from one network event
do not retry in lockstep.

Wake and network-regain short-circuit the wait. Without that, closing a
laptop lid and opening it means sitting through the ceiling while
everything looks broken, which is the most common way this will be
experienced.

notInstalled is a state of its own because it is not a failure to retry:
retrying forever produces noise instead of the one sentence that fixes it.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: `FleetStore`

**Files:**
- Create: `apps/macos/Sources/FarCooler/FleetStore.swift`

**Interfaces:**
- Consumes: `DaemonClient(target:)`, `DaemonClient.state`, `HostState` (Tasks 2-3); `Hosts.shared.all`.
- Produces:
  - `FleetStore.clients: [String: DaemonClient]` (private setter)
  - `FleetStore.fleet: Fleet` — merged, `@Published`
  - `FleetStore.client(for: Workspace) -> DaemonClient?`
  - `FleetStore.state(of host: String) -> HostState`
  - `FleetStore.refusal(for: Workspace) -> String?` — nil when the mutation may proceed
  - `FleetStore.reconnect(_ host: String)` and `reconnectAll()`
  - `FleetStore.hosts: [String]` — every target, local first, then `Hosts.all` order

- [ ] **Step 1: Write it**

```swift
import Combine
import SwiftUI

/// Every machine, at once.
///
/// `Fleet` is one machine's decoded `workspace list --json`; this holds N of
/// them and publishes the merge. Views observe this one object rather than
/// subscribing to a client per machine.
///
/// Membership is always the local machine plus one client per configured host.
/// `Hosts.all` lists remote machines only — this Mac is the implicit entry under
/// the empty-string key, which is the same convention `workspace.host` uses on
/// the wire and `OpenInEditor` already reads.
@MainActor
final class FleetStore: ObservableObject {
    @Published private(set) var fleet: Fleet = .empty
    @Published private(set) var clients: [String: DaemonClient] = [:]

    private var hostsObserver: AnyCancellable?
    private var clientObservers: [String: AnyCancellable] = [:]

    init() {
        rebuild()
        hostsObserver = Hosts.shared.objectWillChange.sink { [weak self] _ in
            // objectWillChange fires BEFORE the array is updated, so read it
            // on the next turn or a machine added here is missed until the one
            // after it.
            Task { @MainActor in self?.rebuild() }
        }
        Reachability.shared.onShouldRetry = { [weak self] in
            self?.reconnectAll()
        }
    }

    /// Every machine, local first.
    var hosts: [String] { [""] + Hosts.shared.all.map(\.target) }

    /// Bring clients into line with the configured machines.
    ///
    /// Adding a machine in Settings brings one up; removing one tears its
    /// client down. Existing clients are kept rather than replaced, because
    /// replacing one would drop a live connection and its fleet with it.
    private func rebuild() {
        let wanted = Set(hosts)

        for target in wanted where clients[target] == nil {
            let client = DaemonClient(target: target)
            clients[target] = client
            clientObservers[target] = client.objectWillChange.sink { [weak self] _ in
                Task { @MainActor in self?.remerge() }
            }
            Task { @MainActor in
                await client.refresh()
                client.startEvents()
            }
        }

        for (target, client) in clients where !wanted.contains(target) {
            client.stopEvents()
            clients[target] = nil
            clientObservers[target] = nil
        }

        remerge()
    }

    /// One list from N.
    ///
    /// An unreachable machine still contributes its last good rows — that is
    /// what keeps your mental map of the fleet stable while a laptop sleeps. A
    /// machine that has never connected contributes none, and appears only as a
    /// header with its state.
    private func remerge() {
        var merged: [Workspace] = []
        var live = 0
        var healthy = false
        for target in hosts {
            guard let client = clients[target] else { continue }
            merged.append(contentsOf: client.fleet.workspaces)
            live += client.fleet.livePanes
            if client.fleet.runtimeHealthy { healthy = true }
        }
        fleet = Fleet(runtimeHealthy: healthy, livePanes: live, workspaces: merged)
    }

    // MARK: - Routing

    /// The machine a row came from.
    ///
    /// By `workspace.host`, which the CLI stamps from the `--host` flag it was
    /// invoked with. Never by id: short ids are the last eight hex of a UUID
    /// minted per daemon, so they say nothing about which machine they are on.
    func client(for workspace: Workspace) -> DaemonClient? {
        clients[workspace.host ?? ""]
    }

    func state(of host: String) -> HostState {
        clients[host]?.state ?? .connecting
    }

    /// Why this row's machine cannot be acted on, or nil if it can.
    ///
    /// Checked here rather than at each call site. There are around a hundred
    /// and twenty of those, and a rule every one of them has to remember is a
    /// rule that gets forgotten — the failure being a command that hangs for
    /// ConnectTimeout against a machine already known to be gone.
    func refusal(for workspace: Workspace) -> String? {
        guard let client = client(for: workspace) else {
            return "this machine is no longer configured"
        }
        return client.state.refusal
    }

    // MARK: - Retrying

    func reconnect(_ host: String) { clients[host]?.reconnectNow() }

    func reconnectAll() { for client in clients.values { client.reconnectNow() } }
}
```

- [ ] **Step 2: Build**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
./apps/macos/build-app.sh
```

Expected: builds. `FleetStore` is not wired into any view yet, so the app is unchanged — Task 5 does that. If the compiler reports `Fleet`'s initialiser is inaccessible, add a memberwise `init` to it in `Model.swift` rather than making `remerge` construct it some other way.

- [ ] **Step 3: Commit**

```bash
git add apps/macos/Sources/FarCooler/FleetStore.swift
git commit -m "feat(macos): FleetStore holds every machine at once

One client per configured machine plus the local one, merged into one
list and routed by workspace.host -- the key the CLI already stamps and
OpenInEditor already reads.

Refusing an action against an unreachable machine lives here rather than
at the call sites. There are around a hundred and twenty of those, and a
rule every one has to remember is a rule that gets forgotten.

Not yet wired into any view.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: `ContentView` routes through `FleetStore`

**Files:**
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift` (~122 call sites, `Selection`)

**Interfaces:**
- Consumes: everything from Task 4.
- Produces: `ContentView.Selection` carries the host:
  ```swift
  enum Selection: Hashable {
      case workspace(host: String, id: String)
      case terminal(host: String, workspace: String, terminal: String)
  }
  ```

- [ ] **Step 1: Swap the state object**

```swift
    @StateObject private var store = FleetStore()
```

Delete `@StateObject private var client = DaemonClient(...)`.

- [ ] **Step 2: Work through the call sites**

The compiler drives this: every `client.` is now an error. Three shapes:

1. **A workspace is in hand** — `client.foo(ws)` becomes:
   ```swift
   if let c = store.client(for: ws) { await c.foo(ws) }
   ```
2. **Reading merged state** — `client.fleet` becomes `store.fleet`.
3. **Per-machine reads with no workspace** (`repositories`, `roots`, `layouts`) — these are per-machine and must not be silently merged. Add to `FleetStore`, following `remerge`'s shape:
   ```swift
   /// Repositories across every machine, each tagged with the machine it is on.
   var repositories: [(host: String, repository: Repository)] {
       hosts.flatMap { host in
           (clients[host]?.repositories ?? []).map { (host, $0) }
       }
   }
   ```
   Do the same for `roots`. For `layouts`, key by `(host, workspace)` — a layout is per-workspace and a workspace is per-machine, so a flat `[String: [PaneGroup]]` across machines can collide.

**Before mutating, check the refusal:**

```swift
    private func act(on ws: Workspace, _ body: (DaemonClient) async -> Void) async {
        if let why = store.refusal(for: ws) {
            errorBanner = "Cannot do that: \(why)"
            return
        }
        guard let client = store.client(for: ws) else { return }
        await body(client)
    }
```

Route every mutation through it. Reads do not go through it — reading last-known state from an unreachable machine is the point.

- [ ] **Step 3: Widen `Selection`**

Change the enum as in Interfaces, then fix each construction and match. Anywhere a selection is resolved back to a `Workspace`, match on host AND id:

```swift
    private func workspace(for selection: Selection) -> Workspace? {
        switch selection {
        case .workspace(let host, let id):
            return store.fleet.workspaces.first { ($0.host ?? "") == host && $0.short == id }
        case .terminal(let host, let workspace, _):
            return store.fleet.workspaces.first { ($0.host ?? "") == host && $0.short == workspace }
        }
    }
```

- [ ] **Step 4: Fix the two `hostArguments` hand-offs**

`ContentView.swift:668` and `:721` pass `client.cliHostArguments` into a pane. They become the pane's own workspace's client:

```swift
    hostArguments: store.client(for: ws)?.cliHostArguments ?? [],
```

- [ ] **Step 5: Build and exercise**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
./apps/macos/build-app.sh
```

With only the local machine configured, everything must behave exactly as before: sidebar populated, terminals open, layouts work, hide/unhide work, removal works. Then add a remote machine in Settings and confirm both machines' projects appear in one list.

Report what you exercised and what you saw.

- [ ] **Step 6: Commit**

```bash
git add apps/macos/Sources/FarCooler/ContentView.swift apps/macos/Sources/FarCooler/FleetStore.swift
git commit -m "feat(macos): the sidebar shows every machine at once

Every mutation now routes to the machine its row came from, and a
selection carries its host: short ids are the last eight hex of a
per-daemon UUID, so across three machines they say nothing about where
they live, and losing that coin flip means acting on the wrong machine.

Mutations against an unreachable machine are refused with that machine's
own error instead of hanging. Reads are not -- showing a terminal's last
known screen while its machine is asleep is the point.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: The machine in the sidebar

**Files:**
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift` (`groups`)
- Modify: `apps/macos/Sources/FarCooler/SidebarViews.swift` (`ProjectHeader`)

**Interfaces:**
- Consumes: `FleetStore.state(of:)`, `FleetStore.reconnect(_:)`.
- Produces: `ProjectHeader` gains `host: String`, `hostState: HostState`, `showHost: Bool`, `onReconnect: () -> Void`.

- [ ] **Step 1: Key the groups by machine and project**

```swift
    /// Worktrees matching the search, grouped by machine and project.
    ///
    /// The key carries the host because two machines can have a project of the
    /// same name, and they are not the same project. The host is only DISPLAYED
    /// when there is more than one machine — on a fleet of one, saying which
    /// machine is noise.
    private var groups: [(host: String, project: String, shown: [Workspace], hidden: [Workspace])] {
        let visible = store.fleet.workspaces.filter { $0.matches(query) }
        var order: [String] = []
        var byKey: [String: [Workspace]] = [:]

        for workspace in visible {
            let host = workspace.host ?? ""
            let project = (workspace.repository ?? "").isEmpty
                ? "Ungrouped" : workspace.repository!
            let key = "\(host)\u{1}\(project)"
            if byKey[key] == nil { order.append(key) }
            byKey[key, default: []].append(workspace)
        }

        return order.map { key in
            let parts = key.split(separator: "\u{1}", maxSplits: 1, omittingEmptySubsequences: false)
            let host = String(parts[0])
            let project = String(parts.count > 1 ? parts[1] : "")
            let all = byKey[key] ?? []
            // Main checkout first, then the daemon's order. A stable partition,
            // not a sort: sorted(by:) is not guaranteed stable and rows would
            // reshuffle on every fleet event.
            let shown = all.filter { !$0.isHidden && $0.isMainCheckout }
                + all.filter { !$0.isHidden && !$0.isMainCheckout }
            return (host, project, shown, all.filter(\.isHidden))
        }
    }

    /// Whether to name machines at all.
    private var showHosts: Bool { store.hosts.count > 1 }
```

Also add a group for a configured machine that contributed no rows, so it is visible rather than absent:

```swift
    /// Machines with nothing to show yet still get a header.
    ///
    /// A machine that has never connected has no rows, and without this it
    /// would simply be missing — leaving you to wonder where it went rather
    /// than seeing that it needs attention.
    private var silentHosts: [String] {
        let present = Set(store.fleet.workspaces.map { $0.host ?? "" })
        return store.hosts.filter { !present.contains($0) }
    }
```

- [ ] **Step 2: Teach `ProjectHeader` about the machine**

Add to `ProjectHeader`:

```swift
    let host: String
    let hostState: HostState
    let showHost: Bool
    let onReconnect: () -> Void
```

and in its body, after the project name:

```swift
                if showHost {
                    Text(host.isEmpty ? "this Mac" : host)
                        .font(.system(size: 10))
                        .foregroundStyle(.tertiary)
                        .lineLimit(1)
                }
                HostDot(state: hostState, onReconnect: onReconnect)
```

- [ ] **Step 3: Write `HostDot`**

In `SidebarViews.swift`:

```swift
/// A machine's connection, said as quietly as possible.
///
/// Absent when healthy: a dot that is always there is a dot nobody reads, and
/// the whole point is that you notice it only when something is wrong.
/// Reconnection is amber and silent; only a machine that has given up is red,
/// and clicking it retries at once rather than waiting out the backoff.
struct HostDot: View {
    let state: HostState
    let onReconnect: () -> Void

    var body: some View {
        switch state {
        case .connected:
            EmptyView()
        case .connecting, .reconnecting:
            Circle()
                .fill(Color.orange)
                .frame(width: 5, height: 5)
                .help("Reconnecting to this machine")
        case .unreachable(let why):
            Button(action: onReconnect) {
                Circle().fill(Color.red).frame(width: 5, height: 5)
            }
            .buttonStyle(.plain)
            .help("\(why) — click to retry now")
        case .notInstalled:
            Button(action: onReconnect) {
                Circle().fill(Color.secondary).frame(width: 5, height: 5)
            }
            .buttonStyle(.plain)
            .help("Far Cooler is not installed on this machine — open Settings ▸ Machines")
        }
    }
}
```

- [ ] **Step 4: Dim an unreachable machine's rows**

Where a `WorkspaceSection` is built, pass whether its machine is usable, and apply `.opacity(0.55)` and `.allowsHitTesting` only to the mutating affordances when it is not. Reads stay live.

- [ ] **Step 5: Build and exercise**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
./apps/macos/build-app.sh
```

| Do this | Expect |
|---|---|
| One machine configured | no host labels, no dots |
| Add a second machine | both named; projects interleaved in one scroll area |
| `farcooler daemon stop --host <remote>` | that machine's dot goes amber then red; its rows stay, dimmed |
| Click the red dot | retries at once |
| Try to hide a worktree on the down machine | refused with that machine's message, no hang |

- [ ] **Step 6: Commit**

```bash
git add apps/macos/Sources/FarCooler/ContentView.swift apps/macos/Sources/FarCooler/SidebarViews.swift
git commit -m "feat(macos): name the machine beside each project

The group key carries the host because two machines can have a project of
the same name and they are not the same project. The name is only shown
when there is more than one machine -- on a fleet of one it is noise.

The state dot is absent when healthy. A dot that is always there is a dot
nobody reads, and the point is noticing only when something is wrong.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Choosing a machine when adding a repository

**Files:**
- Modify: `apps/macos/Sources/FarCooler/Sheets.swift` (`AddRepositorySheet`)
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift` (the sheet's presentation)

**Interfaces:**
- Consumes: `FleetStore.hosts`, `FleetStore.clients`.
- Produces: `AddRepositorySheet` gains `hosts: [String]` and `@State private var host: String`; its `onAddRoot`/`onRegister` closures gain a leading `host` parameter.

- [ ] **Step 1: Add the picker**

In `AddRepositorySheet`, above the folder chooser:

```swift
            if hosts.count > 1 {
                Picker("Machine", selection: $host) {
                    ForEach(hosts, id: \.self) { h in
                        Text(h.isEmpty ? "This Mac" : h).tag(h)
                    }
                }
                .pickerStyle(.menu)
            }
```

- [ ] **Step 2: Make the path field mean the right thing**

A local file browser cannot see a remote machine's disk, so when `host` is non-empty the chooser is replaced by a text field:

```swift
            if host.isEmpty {
                // the existing NSOpenPanel-backed chooser, unchanged
            } else {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Path on \(host)")
                        .font(.callout)
                    TextField("/home/you/src/project", text: $remotePath)
                        .textFieldStyle(.roundedBorder)
                    Text("Checked on that machine when you add it — this Mac cannot see its disk.")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
```

`looksLikeRepository` and `coveringRoot` both consult the local filesystem, so they apply only when `host.isEmpty`. For a remote machine, `canConfirm` is just a non-empty path: the daemon on that machine is the authority and its refusal is the answer.

- [ ] **Step 3: Route the calls**

The closures gain the host and `ContentView` routes them:

```swift
                onAddRoot: { host, path in
                    await store.clients[host]?.addRoot(path) ?? "that machine is not connected"
                },
                onRegister: { host, path in
                    await store.clients[host]?.registerRepository(path) ?? "that machine is not connected"
                },
```

- [ ] **Step 4: Build and exercise**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
./apps/macos/build-app.sh
```

Add a local repository (unchanged flow), then a remote one by typed path, then a deliberately wrong remote path and confirm the daemon's own message comes back.

- [ ] **Step 5: Commit**

```bash
git add apps/macos/Sources/FarCooler/Sheets.swift apps/macos/Sources/FarCooler/ContentView.swift
git commit -m "feat(macos): choose the machine when adding a repository

A local file browser cannot see a remote machine's disk, so choosing a
remote machine turns the chooser into a typed path and hands validation
to the daemon on that machine -- which is the only thing that can answer.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: Delete the picker

**Files:**
- Delete: `apps/macos/Sources/FarCooler/MachinePicker.swift`
- Modify: `apps/macos/Sources/FarCooler/Preferences.swift` (`remoteHost`)
- Modify: `apps/macos/Sources/FarCooler/ContentView.swift` (`sidebarHeader`)
- Modify: `apps/macos/Sources/FarCooler/Hosts.swift` (`active`)

- [ ] **Step 1: Find every reference**

```bash
grep -rn "MachinePicker\|remoteHost\|hosts.active\|Hosts.shared.active" apps/macos/Sources apps/ios 2>/dev/null
```

Anything under `apps/ios` is out of scope — the iOS app has its own host handling and its own phase. Leave it.

- [ ] **Step 2: Remove them**

```bash
git rm apps/macos/Sources/FarCooler/MachinePicker.swift
```

Delete `Preferences.remoteHost`. Delete `Hosts.active` — it is a computed passthrough to that preference, and "which one we are driving" has no meaning once the answer is "all of them". Keep everything else in `Hosts`; it is the configured-machine list, which is what it always was.

In `sidebarHeader`, the `MachinePicker()` is replaced by a plain title. The header used to name the machine because the sidebar was one machine's; now it is every machine's, and each row says which:

```swift
            Text("Fleet")
                .font(.headline)
```

- [ ] **Step 3: Build and check**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
./apps/macos/build-app.sh
```

Expected: builds, no picker, machines still manageable in Settings ▸ Machines.

- [ ] **Step 4: Commit**

```bash
git add -u apps/macos
git commit -m "refactor(macos): delete the machine picker

It was never a filter. It was the --host flag on every CLI call, which is
why switching machines threw away a sidebar full of real rows and built a
different one. Every machine is present now, and each row says which it
is on, so there is nothing left to pick.

Hosts stays: it is the list of configured machines, which is what it
always actually was. Only the selection it carried is gone.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 9: Documentation and the reconnection matrix

**Files:**
- Modify: `docs/farcooler-design.md`
- Modify: `README.md` if it describes the picker

- [ ] **Step 1: Find what the docs still claim**

```bash
grep -rn "This Mac\|remoteHost\|machine picker\|one machine" docs/farcooler-design.md README.md | head -20
```

- [ ] **Step 2: Rewrite the section**

```markdown
Every configured machine is connected at once. Projects from all of them appear
in one list, each naming the machine it is on, and every action routes back to
that machine. There is no "current machine": an agent blocked on a server in
another room is exactly as visible as one on this desk, which was the claim the
picker quietly broke.

A machine that stops answering keeps its rows, dimmed, and says so on its
project headers. Reads still work against the last good fetch; actions are
refused at once with that machine's own error rather than hanging. Nothing is
queued — an action replayed minutes later against changed state is a surprise
nobody can attribute.
```

- [ ] **Step 3: Run the full reconnection matrix**

This is the acceptance test for the whole plan. Run every row and record what happened:

| Case | Expected |
|---|---|
| `farcooler daemon stop` on a remote host | amber within ~1s, reconnects when restarted |
| Drop the network (Wi-Fi off) | amber, then red; rows stay, dimmed |
| Sleep and wake the Mac | reconnects immediately, not after a backoff wait |
| Reboot a remote host | red while down, reconnects unaided |
| Unreachable host, attempt a mutation | refused at once with that host's error, no hang |
| A machine with no daemon installed | `notInstalled`, not a retry loop |
| Three machines, one down | the other two entirely unaffected |

- [ ] **Step 4: Full check**

```bash
export PATH="$HOME/.cargo/bin:$PATH"
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets -- -D warnings
./apps/macos/build-app.sh
```

- [ ] **Step 5: Commit**

```bash
git add docs README.md
git commit -m "docs: every machine in one fleet

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## Self-Review

**Spec coverage:**

| Spec section | Task |
|---|---|
| SSH keepalives and `ConnectTimeout` | 1 |
| `DaemonClient` per machine | 2 |
| Backoff, jitter, wake/network short-circuit | 3 |
| Connection states incl. `notInstalled` | 3 |
| `FleetStore`: membership, merge, routing | 4 |
| Refusal centralised, not per call site | 4, 5 |
| `Selection` carries the host | 5 |
| Group key `(host, project)`; host shown only when >1 | 6 |
| Quiet dot; click red to retry | 6 |
| Unreachable rows stay, dimmed, readable | 6 |
| Machine that never connected still visible | 6 |
| Add Repository host picker | 7 |
| Delete picker, `remoteHost`, `hostChanged` | 2, 8 |
| Reconnection matrix | 9 |
| iOS phase two | not in this plan, by design |

**Known soft spots, stated rather than hidden:**

- **Task 5 is much larger than the others.** ~122 call sites in one file. It cannot be split usefully — the compiler breaks all of them at once the moment the state object changes — but a reviewer should expect a big diff and should read `act(on:_:)`'s use at every mutation rather than skimming.
- **No automated test covers any Swift here.** Every Swift task's verification is the build plus a manual checklist. That is a real gap, and the matrix in Task 9 is the only thing standing in for a test suite. An implementer that reports a checklist row as passing without having performed it has defeated the plan's only verification.
- **`Hosts.objectWillChange` fires before its array updates**, so `rebuild()` is deliberately deferred one turn (Task 4, Step 1). If a machine added in Settings does not appear until another change lands, that deferral is the first place to look.
