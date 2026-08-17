import Combine
import SwiftUI

/// Every runner, at once.
///
/// `Fleet` is one runner's decoded `workspace list --json`; this holds N of
/// them and publishes the merge. Views observe this one object rather than
/// subscribing to a client per runner.
///
/// Membership is always the local runner plus one client per configured runner.
/// `Runners.all` lists remote runners only — this Mac is the implicit entry
/// under the empty-string key, which is the same convention `workspace.host`
/// uses on the wire and `OpenInEditor` already reads. The keys here stay
/// host-shaped for that reason: they are ssh targets, matching the wire.
@MainActor
final class FleetStore: ObservableObject {
    @Published private(set) var fleet: Fleet = .empty
    @Published private(set) var clients: [String: DaemonClient] = [:]

    private var runnersObserver: AnyCancellable?
    private var clientObservers: [String: AnyCancellable] = [:]
    /// The Task that brings a freshly-added client up: its first `refresh()`,
    /// then its event stream.
    ///
    /// Tracked for the same reason `DaemonClient.retryTask` is: without a
    /// slot to cancel, removing a runner while this Task is still awaiting
    /// its first `refresh()` cannot stop it. `stopEvents()` in the removal
    /// loop below is a no-op against a client that has not started anything
    /// yet, and the Task, still holding a strong reference to that client,
    /// resumes once `refresh()` returns and calls `startEvents()` regardless
    /// — spawning `farcooler --host <removed> events` against a runner the
    /// user just deleted. Cancelling this slot on removal, and checking
    /// `Task.isCancelled` before that `startEvents()` call, closes the gap.
    ///
    /// Same invariant as `retryTask`: a cancelled task always leaves its slot
    /// nil, so `!= nil` here means "still bringing up", never "used to be".
    private var bringUpTasks: [String: Task<Void, Never>] = [:]

    init() {
        rebuild()
        runnersObserver = Runners.shared.objectWillChange.sink { [weak self] _ in
            // objectWillChange fires BEFORE the array is updated, so read it
            // on the next turn or a runner added here is missed until the one
            // after it.
            Task { @MainActor in self?.rebuild() }
        }
        Reachability.shared.onShouldRetry = { [weak self] in
            self?.reconnectAll()
        }
    }

    /// Every runner, local first.
    var hosts: [String] { [""] + Runners.shared.all.map(\.target) }

    /// Resume every client's event stream.
    ///
    /// `rebuild()` starts a fresh client's stream exactly once, at the
    /// moment it is added — a client already running is none of its
    /// business. But `ContentView`'s `.onDisappear` stops every stream on
    /// the way out, and before this there was nothing symmetric on the way
    /// back in: `.task` used to call `client.startEvents()` itself, and
    /// delegating startup to `rebuild()` dropped that call along with the
    /// rest of it. A window that closes and reopens while this store's own
    /// lifetime spans both — the common case, since closing the last window
    /// does not quit the app — came back with `state` still reading
    /// `.connected` and a green dot, but no stream, no retry and no timer
    /// underneath it. `startEvents()`'s own `eventStream == nil` guard makes
    /// this safe to call unconditionally: for the ordinary case, where
    /// `rebuild()` already started every client, every one of these is a
    /// no-op.
    func resume() {
        for client in clients.values { client.startEvents() }
    }

    /// Bring clients into line with the configured runners.
    ///
    /// Adding a runner in Settings brings one up; removing one tears its
    /// client down. Existing clients are kept rather than replaced, because
    /// replacing one would drop a live connection and its fleet with it.
    private func rebuild() {
        let wanted = Set(hosts)

        for target in wanted where clients[target] == nil {
            let client = DaemonClient(target: target)
            clients[target] = client
            // Assigned here, before the bring-up task below has awaited
            // anything, so it is in place no matter how early
            // `cancelBringUp()` cuts that task short — a click on a red
            // trouble dot, `reconnectAll()`, or the runner simply coming
            // back on its own. Assigned any later and a bring-up cancelled
            // before reaching that point loses this callback for good: the
            // client keeps running (`DaemonClient`'s own retry loop owns
            // that independently of this task), but nothing is left to
            // re-seed repositories, roots or layouts when it eventually
            // reconnects. That used to be the case the callback itself was
            // added to fix.
            //
            // Firing twice on the very first connect is not a risk this
            // ordering creates: `refresh()` below only fires `onReconnect`
            // on a genuine transition into `.connected`, and every client
            // starts in `.connecting`, so its first successful `refresh()`
            // — whichever caller makes it, bring-up or otherwise — is that
            // transition and seeds repositories, roots and layouts on its
            // own. Bring-up does not need to read them itself afterward.
            client.onReconnect = { [weak self] in
                Task { @MainActor in await self?.seed(target) }
            }
            clientObservers[target] = client.objectWillChange.sink { [weak self] _ in
                Task { @MainActor in self?.remerge() }
            }
            bringUpTasks[target] = Task { @MainActor [weak self] in
                // A daemon going away is also the moment another build could
                // take the socket, so the local runner claims it back
                // before its first read — the same rule `DaemonClient`'s own
                // retry loop follows in `scheduleRetry()` and
                // `reconnectNow()`. Skipped for a remote target: only this
                // Mac bundles and starts its own daemon.
                if target.isEmpty { await LocalDaemon.shared.ensure() }
                await client.refresh()
                guard !Task.isCancelled else { return }
                client.startEvents()
                self?.bringUpTasks[target] = nil
            }
        }

        for (target, client) in clients where !wanted.contains(target) {
            // Cancel-and-nil together, same as every other site that retires
            // one of these slots: a cancelled task left non-nil would read
            // as "still bringing up" to anything checking this dictionary.
            bringUpTasks[target]?.cancel()
            bringUpTasks[target] = nil
            client.onReconnect = nil
            client.stopEvents()
            clients[target] = nil
            clientObservers[target] = nil
        }

        remerge()
    }

    /// Re-read repositories, roots and layouts after a reconnection — the
    /// same three reads that seed a runner the first time, run again
    /// because a runner that drops and comes back must not stay invisible
    /// to the project pickers and root checks until the app relaunches. See
    /// `DaemonClient.onReconnect`, which this is wired to for every
    /// connection a client makes, bring-up's own first one included.
    ///
    /// Sequential, with a cancellation check between each: a runner removed
    /// (its client torn down by `rebuild()`) or reconnected again out from
    /// under this exact seed stops firing the next subprocess rather than
    /// running all three regardless.
    private func seed(_ target: String) async {
        guard let client = clients[target] else { return }
        await client.refreshRepositories()
        guard clients[target] === client else { return }
        await client.refreshRoots()
        guard clients[target] === client else { return }
        await client.refreshLayouts()
        guard clients[target] === client else { return }
        // Themes, for the same reason as the three above: a runner that
        // dropped and came back may have gained one, and a `[themes.*]` table
        // added to config.toml should not stay invisible until the app is
        // relaunched. Only the local runner's, because a theme is what THIS
        // app paints with — asking three remote hosts and merging their
        // answers would make the picker's contents depend on which runners
        // happen to be awake.
        if target.isEmpty {
            await Themes.shared.reload(
                binary: client.cliPath, environment: client.cliEnvironment, host: [])
        }
    }

    /// One list from N.
    ///
    /// An unreachable runner still contributes its last good rows — that is
    /// what keeps your mental map of the fleet stable while a laptop sleeps. A
    /// runner that has never connected contributes none, and appears only as a
    /// header with its state.
    private func remerge() {
        var merged: [Workspace] = []
        var live = 0
        var healthy = false
        for target in hosts {
            guard let client = clients[target] else { continue }
            merged.append(contentsOf: client.fleet.workspaces)
            // Not counted once a runner is known unreachable or never
            // installed: `client.fleet.livePanes` and `client.fleet.runtimeHealthy`
            // are then whatever was last read before it went quiet, not a live
            // reading, and letting either in reports hands on deck — or tmux
            // health — that in fact went home. Without this gate on `healthy`,
            // a single unreachable runner with a stale `runtimeHealthy == true`
            // could paint the whole bar green while the only runner actually
            // answering has no tmux at all; on a fleet of one, the sole
            // daemon dying would still show a green dot and "0 live".
            if client.state.refusal == nil {
                live += client.fleet.livePanes
                if client.fleet.runtimeHealthy { healthy = true }
            }
        }
        fleet = Fleet(runtimeHealthy: healthy, livePanes: live, workspaces: merged)
    }

    /// Runners that are not fully healthy right now, for the status bar to
    /// name individually.
    ///
    /// `fleet.runtimeHealthy` above ORs across every runner, and stays an OR
    /// on purpose — ANDing would paint the whole status bar orange every time
    /// any one laptop was merely asleep, which is not news worth a color
    /// change. But a single merged boolean is also the whole story only if
    /// nobody needs to know WHICH runner is the problem, and with more than
    /// one runner configured that is exactly the question a green dot can no
    /// longer answer: it takes only one healthy runner to turn it green
    /// while a second sits there with no tmux at all. This is how the bar can
    /// name that second runner instead of just going quiet about it.
    ///
    /// A runner still in `.connecting` — the few seconds before its first
    /// read has come back — is not yet known to be anything, so it is left
    /// out rather than reported as broken before it has had a chance to say
    /// otherwise.
    var unhealthyHosts: [String] {
        hosts.filter { host in
            switch clients[host]?.state ?? .connecting {
            case .connecting:
                return false
            case .connected:
                return !(clients[host]?.fleet.runtimeHealthy ?? true)
            case .reconnecting, .unreachable, .notInstalled:
                return true
            }
        }
    }

    // MARK: - Routing

    /// The runner a row came from.
    ///
    /// By `workspace.host`, which the CLI stamps from the `--host` flag it was
    /// invoked with. Never by id: short ids are the last eight hex of a UUID
    /// minted per daemon, so they say nothing about which runner they are on.
    func client(for workspace: Workspace) -> DaemonClient? {
        clients[workspace.host ?? ""]
    }

    func state(of host: String) -> HostState {
        clients[host]?.state ?? .connecting
    }

    // MARK: - Per-runner reads

    /// Repositories across every runner, each tagged with the runner it is on.
    ///
    /// Not merged into a flat `[Repository]`: a repository's own short id is
    /// eight hex characters minted per daemon, same as a workspace's, so two
    /// runners can hand back the same one for two different repositories.
    var repositories: [(host: String, repository: Repository)] {
        hosts.flatMap { host in
            (clients[host]?.repositories ?? []).map { (host, $0) }
        }
    }

    /// Allowlisted roots across every runner, tagged the same way.
    var roots: [(host: String, root: RepositoryRoot)] {
        hosts.flatMap { host in
            (clients[host]?.roots ?? []).map { (host, $0) }
        }
    }

    /// The root a repository lives under, and every other repository on the
    /// same host that shares it.
    ///
    /// Removing a repository actually removes its whole root — the daemon
    /// has no narrower operation — so anything sharing that root goes with
    /// it. A caller has to know that before it can say so honestly rather
    /// than after the fact. `nil` means the root this repository was
    /// supposedly registered under is not in `roots` — stale data rather
    /// than an ordinary case, and worth refusing rather than guessing at.
    func rootAndSiblings(of repository: Repository, host: String) -> (root: RepositoryRoot, siblings: [Repository])? {
        guard let root = roots.first(where: { $0.host == host && $0.root.id == repository.repositoryRootId })
        else { return nil }
        let siblings = repositories
            .filter {
                $0.host == host && $0.repository.repositoryRootId == repository.repositoryRootId
                    && $0.repository.id != repository.id
            }
            .map(\.repository)
        return (root.root, siblings)
    }

    /// One workspace's layout is per-runner as well as per-workspace, so the
    /// key carries both — a flat `[String: [PaneGroup]]` across several
    /// runners could let one runner's layout answer for another's
    /// workspace of the same id.
    struct LayoutKey: Hashable {
        var host: String
        var workspace: String
    }

    /// Every runner's layouts, merged. Read through `client(for:)` when a
    /// workspace is already in hand — this exists for the one place that has
    /// to watch every runner's layouts at once regardless of which is
    /// selected: `ContentView`'s `.onChange(of:)` that keeps the app looking
    /// at wherever tmux just moved focus to.
    var layouts: [LayoutKey: [PaneGroup]] {
        var merged: [LayoutKey: [PaneGroup]] = [:]
        for host in hosts {
            guard let client = clients[host] else { continue }
            for (workspace, groups) in client.layouts {
                merged[LayoutKey(host: host, workspace: workspace)] = groups
            }
        }
        return merged
    }

    /// Why this row's runner cannot be acted on, or nil if it can.
    ///
    /// Checked here rather than at each call site. There are around a hundred
    /// and twenty of those, and a rule every one of them has to remember is a
    /// rule that gets forgotten — the failure being a command that hangs for
    /// ConnectTimeout against a runner already known to be gone.
    func refusal(for workspace: Workspace) -> String? {
        refusal(for: workspace.host ?? "")
    }

    /// Same check, from a bare host rather than a workspace already on it —
    /// for the handful of mutations (new task, resume branch, add root,
    /// register repository, new workspace from the sidebar's own `+`) that
    /// have no workspace in hand yet to route by.
    func refusal(for host: String) -> String? {
        guard let client = clients[host] else {
            return "this runner is no longer configured"
        }
        return client.state.refusal
    }

    // MARK: - Retrying

    /// Cancel any bring-up still in flight for `host` before reconnecting.
    ///
    /// Without this, a runner whose bring-up `refresh()` is still awaiting
    /// its first response and a `reconnectNow()` fired at the same target —
    /// a click on its trouble dot, or `reconnectAll()` — end up running two
    /// `refresh()` + `startEvents()` sequences on the one client at once,
    /// each free to overwrite what the other just set.
    private func cancelBringUp(_ host: String) {
        bringUpTasks[host]?.cancel()
        bringUpTasks[host] = nil
    }

    func reconnect(_ host: String) {
        cancelBringUp(host)
        clients[host]?.reconnectNow()
    }

    /// Retry every client that is not currently connected.
    ///
    /// Not every client — a healthy one included would retry in lockstep
    /// with no jitter (the jitter in `DaemonClient.backoffSeconds` exists
    /// precisely to avoid this), and would tear down and restart the local
    /// runner's own perfectly good event stream on every Wi-Fi change,
    /// which is the one runner a network flap never actually touches.
    func reconnectAll() {
        for (host, client) in clients where !client.state.isUsable {
            cancelBringUp(host)
            client.reconnectNow()
        }
    }
}
