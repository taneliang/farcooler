import Combine
import SwiftUI

/// One workspace, the runner it is on, and what that runner said about its
/// diff.
///
/// Android's `FleetEntry`, on a phone that had no equivalent because it never
/// had more than one runner to merge. The runner is CARRIED rather than looked
/// up, and that is the whole reason this type exists instead of a bare
/// `[Workspace]`: a workspace id is the last eight hex characters of a UUID
/// minted per daemon, so across three runners two of them can hand back the
/// same id for two unrelated worktrees, and acting on the wrong one is not a
/// failure anything would report.
struct FleetEntry: Identifiable {
    let host: Runner
    let connection: Connection
    let workspace: Workspace
    /// This worktree's `changes.inbox` row, or nil while its runner has not
    /// answered. Carried here rather than looked up by the screens that want
    /// it, for the same reason the runner is: this is the app's one merged
    /// list, and a screen reaching back into a per-connection map would have to
    /// re-key every one of them by runner itself.
    let counts: InboxRow?

    /// Unique across the fleet, which `workspace.id` is not. See the note
    /// above, and `ShellFleetMap.tabID` — which has exactly this bug waiting
    /// for it and is step 4 of the port rather than this one.
    var id: String { "\(host.id.uuidString)/\(workspace.id)" }
}

/// Every configured runner, connected at once.
///
/// The Mac's `FleetStore` and Android's `FleetRepository`, on the one platform
/// that made the runner a mode. It holds one `Connection` per runner keyed by
/// runner id, brings connections up and down as the runner list changes, and
/// publishes the merge. Views observe this one object rather than a connection
/// each.
///
/// **Nothing constructs this yet, and that is deliberate.** It is step 1 of a
/// nine-step port (`.claude/agent/done/the-fifth-cost-of-the-multi-runner-port.md`),
/// and steps 5 to 7 are what make it safe to wire in. Until they land, this
/// store starting N connections would have every one of them fight over the
/// three process-wide slots `Connection.start` still claims:
///
/// - `WatchLinkHost.shared.adopt` — the watch would perform everything through
///   whichever runner happened to start last.
/// - `Connection.current` — the enrollment ceremony writes `authorized_keys`
///   through that slot, so the same lottery would decide which runner a device
///   gets added to.
/// - the one `fleet.json` — whichever connection polled last would define the
///   whole lock screen's fleet.
///
/// `Reachability` is NOT on that list any more: it holds a list of subscribers
/// keyed by runner id rather than one slot, so every connection here is woken
/// by one network change. Each connection subscribes for itself in `start` and
/// `retire` removes the entry; when the rest of the port lands, the right move
/// is the one both other platforms already made — this store becomes the single
/// subscriber and fans out — but a list with one entry per connection costs
/// nothing and is not worth changing ahead of the screens.
///
/// **Failure isolation is structural**, which is the argument for a connection
/// per runner over one connection taking a runner argument: one runner being
/// unreachable is one object in a bad state rather than a flag threaded through
/// shared code, so there is somewhere natural for "this runner is down, here is
/// why" to live. That somewhere is `RunnerStatusRow`.
@MainActor
final class FleetStore: ObservableObject {
    /// Every workspace on every connected runner, in runner order.
    @Published private(set) var entries: [FleetEntry] = []

    /// One per runner currently being talked to, in the order they are listed.
    @Published private(set) var active: [Connection] = []

    private let hosts: RunnerStore

    private var connections: [UUID: Connection] = [:]

    /// The details each connection was DIALED with.
    ///
    /// Not read back off the connection, and not the same thing as the runner
    /// in `hosts` today: this is what makes an edit detectable at all. See
    /// `FleetMembership.plan`, which compares the two and rebuilds when they
    /// differ, because reusing a connection after its address changed would
    /// leave the old session running under new details.
    private var dialed: [UUID: Runner] = [:]

    /// The Task bringing a freshly-added connection up.
    ///
    /// Tracked so that removing a runner mid-connect can stop it. `start`
    /// cannot be interrupted — the SSH attempt is a ticket the core resolves
    /// whenever the network gets round to it, and a routable address with
    /// nothing listening takes over a minute to fail — so what cancelling buys
    /// is this store no longer caring about the answer.
    ///
    /// Same invariant the Mac's `bringUpTasks` keeps: a cancelled task always
    /// leaves its slot nil, so a non-nil entry means "still bringing up" and
    /// never "used to be".
    private var starts: [UUID: Task<Void, Never>] = [:]

    /// What is watching each connection, so retiring one takes its watcher with
    /// it.
    private var watchers: [UUID: AnyCancellable] = [:]

    private var runnersObserver: AnyCancellable?

    /// Whether to talk to every configured runner, or only the selected one.
    ///
    /// Hard `true` for one commit only. It becomes a setting in step 2 of the
    /// port, mirroring Android's `Settings.allRunnersAtOnce` and its default —
    /// see `FleetMembership.wanted`, which is where the choice is already
    /// honored and already pinned by a test.
    private let everyRunnerAtOnce = true

    init(hosts: RunnerStore) {
        self.hosts = hosts
        reconcile()
        runnersObserver = hosts.objectWillChange.sink { [weak self] _ in
            // `objectWillChange` fires BEFORE the array is updated, so this has
            // to read it on the next turn or a runner added here is missed
            // until the one after it. The Mac's `FleetStore.init` learned this
            // the same way.
            Task { @MainActor in self?.reconcile() }
        }
    }

    deinit {
        // Not `retire()` on each: this object is going away with everything it
        // owns, and `retire` hops onto the main actor. What must not outlive it
        // is the bring-up tasks, which hold a strong reference to their
        // connection and would otherwise run a connect against a runner nobody
        // is looking at.
        for task in starts.values { task.cancel() }
    }

    // MARK: - Membership

    /// Bring connections into line with the runners that are wanted.
    ///
    /// Adding a runner in settings brings one up and removing one tears its
    /// connection down, with no relaunch; turning the battery gate off retires
    /// every connection but the selected runner's. The decisions are
    /// `FleetMembership`'s, in AgentKit, because the iOS target's tests are
    /// compiled by CI and never run — see that file's header.
    private func reconcile() {
        let wanted = FleetMembership.wanted(
            all: hosts.hosts, selected: hosts.selected?.id,
            everyRunnerAtOnce: everyRunnerAtOnce)
        let plan = FleetMembership.plan(wanted: wanted, existing: dialed)

        // Retired first, and rebuilt runners retired here rather than replaced
        // in place: a `Connection` assigned over would be freed at some point
        // after its successor had claimed the same runner id, and the loser's
        // `Reachability` entry is keyed by that id.
        for id in plan.retired { retire(id) }
        for id in plan.rebuilt { retire(id) }

        // `kept` is not iterated, and that is the point of it being a list of
        // its own: a connection under unchanged details is left completely
        // alone. Replacing one would drop a live connection and its fleet — the
        // Mac's `rebuild()` names this as the reason it keeps rather than
        // replaces — and it would look identical on screen a second later.
        let byID = Dictionary(hosts.hosts.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        for id in plan.started + plan.rebuilt {
            guard let runner = byID[id] else { continue }
            bringUp(runner)
        }

        publish()
    }

    private func bringUp(_ runner: Runner) {
        let connection = Connection()
        connections[runner.id] = connection
        dialed[runner.id] = runner
        watchers[runner.id] = connection.objectWillChange.sink { [weak self] _ in
            // Next turn, for `objectWillChange`'s reason again: read here, the
            // fleet and the phase are still the previous ones.
            Task { @MainActor in self?.publish() }
        }
        starts[runner.id] = Task { @MainActor [weak self] in
            await connection.start(host: runner)
            guard !Task.isCancelled else { return }
            self?.starts[runner.id] = nil
        }
    }

    private func retire(_ id: UUID) {
        // Cancel-and-remove together, so a cancelled task is never left in the
        // dictionary reading as "still bringing up".
        starts.removeValue(forKey: id)?.cancel()
        watchers[id] = nil
        dialed[id] = nil
        connections.removeValue(forKey: id)?.retire()
    }

    // MARK: - The merge

    /// One list from N, in the order the runners are listed.
    ///
    /// Order comes from the runner list a person arranged, never from the
    /// dictionary's own, so what is on screen does not reshuffle when a laptop
    /// wakes up. A runner with no connection contributes nothing rather than a
    /// gap, and a connection under an id that has left the runner list cannot
    /// contribute at all — see `FleetMembership.published`, which is where both
    /// of those are pinned.
    ///
    /// An unreachable runner still contributes its last good rows. That is what
    /// keeps a mental map of the fleet stable while a laptop sleeps, it is what
    /// both other platforms do, and `RunnerStatusRow` is what explains why those
    /// rows are stale.
    private func publish() {
        let ordered = FleetMembership.published(order: hosts.hosts.map(\.id), live: connections)
        active = ordered

        let byID = Dictionary(hosts.hosts.map { ($0.id, $0) }, uniquingKeysWith: { first, _ in first })
        entries = ordered.flatMap { connection -> [FleetEntry] in
            guard let id = connection.hostId, let host = byID[id] else { return [] }
            let counts = connection.inbox
            return connection.fleet.workspaces.map { workspace in
                FleetEntry(
                    host: host, connection: connection, workspace: workspace,
                    counts: counts[workspace.id])
            }
        }
    }

    // MARK: - Routing

    /// The connection for a runner, or nil if that runner is not being talked
    /// to — which the battery gate makes an ordinary answer rather than an
    /// error.
    ///
    /// By runner id and never by workspace id, for `FleetEntry.id`'s reason:
    /// short ids are minted per daemon and say nothing about which runner they
    /// are on.
    func connection(for host: UUID) -> Connection? { connections[host] }

    func connection(for entry: FleetEntry) -> Connection? { connections[entry.host.id] }

    // MARK: - Acting on all of them

    /// Retry one runner that failed, without disturbing the others.
    ///
    /// The reason a fleet needs this at all: on a single-connection app "try
    /// again" and "reconnect everything" were the same act, and here they are
    /// not — retrying the runner in the spare room must not cost a reconnect on
    /// the one being read.
    func retry(_ host: UUID) {
        guard let connection = connections[host], let runner = dialed[host] else { return }
        starts.removeValue(forKey: host)?.cancel()
        starts[host] = Task { @MainActor [weak self] in
            await connection.start(host: runner)
            guard !Task.isCancelled else { return }
            self?.starts[host] = nil
        }
    }

    /// Refresh every runner at once, for pull-to-refresh.
    ///
    /// The counts come too. Somebody pulling the list down is asking for
    /// everything on it, and the diff numbers ride a slower cadence than the
    /// fleet — so without this they are the one thing on screen a pull would
    /// not update.
    func refreshAll() async {
        for connection in active {
            await connection.refresh()
            await connection.loadInbox()
        }
    }

    /// The app came to the foreground, or left it — for every runner.
    ///
    /// One scene phase, N connections. Each one owns its own poller and its own
    /// claim of attention on its runner, so this is a fan-out and not a
    /// broadcast anything shares.
    func setActive(_ active: Bool) {
        for connection in connections.values { connection.setActive(active) }
    }
}
