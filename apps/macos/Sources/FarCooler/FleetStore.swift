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
    /// The Task that brings a freshly-added client up: its first `refresh()`,
    /// then its event stream.
    ///
    /// Tracked for the same reason `DaemonClient.retryTask` is: without a
    /// slot to cancel, removing a machine while this Task is still awaiting
    /// its first `refresh()` cannot stop it. `stopEvents()` in the removal
    /// loop below is a no-op against a client that has not started anything
    /// yet, and the Task, still holding a strong reference to that client,
    /// resumes once `refresh()` returns and calls `startEvents()` regardless
    /// — spawning `farcooler --host <removed> events` against a machine the
    /// user just deleted. Cancelling this slot on removal, and checking
    /// `Task.isCancelled` before that `startEvents()` call, closes the gap.
    ///
    /// Same invariant as `retryTask`: a cancelled task always leaves its slot
    /// nil, so `!= nil` here means "still bringing up", never "used to be".
    private var bringUpTasks: [String: Task<Void, Never>] = [:]

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
            // Assigned here, before the bring-up task below has awaited
            // anything, so it is in place no matter how early
            // `cancelBringUp()` cuts that task short — a click on a red
            // trouble dot, `reconnectAll()`, or the machine simply coming
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
                // take the socket, so the local machine claims it back
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
    /// same three reads that seed a machine the first time, run again
    /// because a machine that drops and comes back must not stay invisible
    /// to the project pickers and root checks until the app relaunches. See
    /// `DaemonClient.onReconnect`, which this is wired to for every
    /// connection a client makes, bring-up's own first one included.
    ///
    /// Sequential, with a cancellation check between each: a machine removed
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
            // Not counted once a machine is known unreachable or never
            // installed: `client.fleet.livePanes` and `client.fleet.runtimeHealthy`
            // are then whatever was last read before it went quiet, not a live
            // reading, and letting either in reports hands on deck — or tmux
            // health — that in fact went home. Without this gate on `healthy`,
            // a single unreachable machine with a stale `runtimeHealthy == true`
            // could paint the whole bar green while the only machine actually
            // answering has no tmux at all; on a fleet of one, the sole
            // daemon dying would still show a green dot and "0 live".
            if client.state.refusal == nil {
                live += client.fleet.livePanes
                if client.fleet.runtimeHealthy { healthy = true }
            }
        }
        fleet = Fleet(runtimeHealthy: healthy, livePanes: live, workspaces: merged)
    }

    /// Machines that are not fully healthy right now, for the status bar to
    /// name individually.
    ///
    /// `fleet.runtimeHealthy` above ORs across every machine, and stays an OR
    /// on purpose — ANDing would paint the whole status bar orange every time
    /// any one laptop was merely asleep, which is not news worth a colour
    /// change. But a single merged boolean is also the whole story only if
    /// nobody needs to know WHICH machine is the problem, and with more than
    /// one machine configured that is exactly the question a green dot can no
    /// longer answer: it takes only one healthy machine to turn it green
    /// while a second sits there with no tmux at all. This is how the bar can
    /// name that second machine instead of just going quiet about it.
    ///
    /// A machine still in `.connecting` — the few seconds before its first
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

    // MARK: - Per-machine reads

    /// Repositories across every machine, each tagged with the machine it is on.
    ///
    /// Not merged into a flat `[Repository]`: a repository's own short id is
    /// eight hex characters minted per daemon, same as a workspace's, so two
    /// machines can hand back the same one for two different repositories.
    var repositories: [(host: String, repository: Repository)] {
        hosts.flatMap { host in
            (clients[host]?.repositories ?? []).map { (host, $0) }
        }
    }

    /// Allowlisted roots across every machine, tagged the same way.
    var roots: [(host: String, root: RepositoryRoot)] {
        hosts.flatMap { host in
            (clients[host]?.roots ?? []).map { (host, $0) }
        }
    }

    /// One workspace's layout is per-machine as well as per-workspace, so the
    /// key carries both — a flat `[String: [PaneGroup]]` across several
    /// machines could let one machine's layout answer for another's
    /// workspace of the same id.
    struct LayoutKey: Hashable {
        var host: String
        var workspace: String
    }

    /// Every machine's layouts, merged. Read through `client(for:)` when a
    /// workspace is already in hand — this exists for the one place that has
    /// to watch every machine's layouts at once regardless of which is
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

    /// Why this row's machine cannot be acted on, or nil if it can.
    ///
    /// Checked here rather than at each call site. There are around a hundred
    /// and twenty of those, and a rule every one of them has to remember is a
    /// rule that gets forgotten — the failure being a command that hangs for
    /// ConnectTimeout against a machine already known to be gone.
    func refusal(for workspace: Workspace) -> String? {
        refusal(for: workspace.host ?? "")
    }

    /// Same check, from a bare host rather than a workspace already on it —
    /// for the handful of mutations (new task, resume branch, add root,
    /// register repository, new workspace from the sidebar's own `+`) that
    /// have no workspace in hand yet to route by.
    func refusal(for host: String) -> String? {
        guard let client = clients[host] else {
            return "this machine is no longer configured"
        }
        return client.state.refusal
    }

    // MARK: - Retrying

    /// Cancel any bring-up still in flight for `host` before reconnecting.
    ///
    /// Without this, a machine whose bring-up `refresh()` is still awaiting
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
    /// machine's own perfectly good event stream on every Wi-Fi change,
    /// which is the one machine a network flap never actually touches.
    func reconnectAll() {
        for (host, client) in clients where !client.state.isUsable {
            cancelBringUp(host)
            client.reconnectNow()
        }
    }
}
