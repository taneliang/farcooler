import Combine
import SwiftUI

/// One workspace, the runner it is on, and what that runner said about its
/// diff.
///
/// Android's `FleetEntry`, on a phone that had no equivalent because it never
/// had more than one runner to merge. The runner is CARRIED rather than looked
/// up, and that is the whole reason this type exists instead of a bare
/// `[Workspace]`: a merged list has to get from a workspace back to the session
/// that can act on it, and looking that up by searching every connection for a
/// matching id is a search that answers with the FIRST match rather than with
/// nothing when it is wrong.
///
/// **The reason originally recorded here was that ids collide, and that is
/// wrong about this app.** `Workspace.id` decodes the daemon's full UUIDv7 —
/// `uuid_of(&w.id).to_string()` in `crates/client/src/session.rs` — and the
/// eight hex characters are the separate `short` field, which nothing in these
/// apps uses as an identity. Two daemons' UUIDv7s differ in 62 random bits. The
/// carrying is right for the reason above; it is not a fix for a collision that
/// happens. See `ShellIdentity`.
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

    /// Unique across the fleet, and says which runner it is on — which
    /// `workspace.id` does not.
    ///
    /// The same composition `ShellIdentity.workspace` makes, deliberately: the
    /// store's merged list and the shell's fleet have to agree on what one
    /// workspace is called, or a card tapped in the overview and the pane it
    /// opens are two different lookups.
    var id: String {
        ShellIdentity.workspace(runner: host.id.uuidString, workspace: workspace.id)
    }
}

/// Every configured runner, connected at once.
///
/// The Mac's `FleetStore` and Android's `FleetRepository`, on the one platform
/// that made the runner a mode. It holds one `Connection` per runner keyed by
/// runner id, brings connections up and down as the runner list changes, and
/// publishes the merge. Views observe this one object rather than a connection
/// each.
///
/// **This is what the app connects through.** `ConnectedRoot` owns one, every
/// screen reads it, and nothing else owns a `Connection` at all.
///
/// How many runners it dials is `FleetSettings.allRunnersAtOnce`'s answer,
/// which defaults on: every configured runner, at once. Turned off it is the
/// selected runner alone, which is the phone on a train paying for one SSH
/// session instead of six.
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

    /// The runners to publish in the order of, which is `hosts` in the app and
    /// a canned list in the layout harness. See `standIn(on:host:)`.
    private var runnerOrder: [Runner] {
        #if DEBUG
        standInOrder.isEmpty ? hosts.hosts : standInOrder
        #else
        hosts.hosts
        #endif
    }

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
    private var settingObserver: AnyCancellable?

    /// The gate's last known value, kept so the notification below can tell a
    /// change from the hundred other things that write to `UserDefaults`.
    /// `UserDefaults.didChangeNotification` fires for every one of them, and
    /// reconciling on each would be harmless but would republish two arrays
    /// per keystroke in the font-size slider.
    private var everyRunnerAtOnce: Bool

    init(hosts: RunnerStore) {
        self.hosts = hosts
        self.everyRunnerAtOnce = FleetSettings.allRunnersAtOnce
        reconcile()
        runnersObserver = hosts.objectWillChange.sink { [weak self] _ in
            // `objectWillChange` fires BEFORE the array is updated, so this has
            // to read it on the next turn or a runner added here is missed
            // until the one after it. The Mac's `FleetStore.init` learned this
            // the same way.
            Task { @MainActor in self?.reconcile() }
        }
        settingObserver = NotificationCenter.default
            .publisher(for: UserDefaults.didChangeNotification)
            .sink { [weak self] _ in
                Task { @MainActor in self?.gateChanged() }
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

    private func gateChanged() {
        let now = FleetSettings.allRunnersAtOnce
        guard now != everyRunnerAtOnce else { return }
        everyRunnerAtOnce = now
        reconcile()
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
        // Which runner each connection is for is THIS store's own key, and is
        // deliberately not read back off the connection.
        //
        // `Connection.hostId` is nil until `start(host:)` has run, so pairing
        // by it drops every connection in the window between being brought up
        // and being dialed — and drops a canned one forever, which is what a
        // layout harness stands on. Worse than either: it is a second answer to
        // "which runner is this", and the two can only ever agree.
        let mine = runnerOrder.compactMap { host in connections[host.id].map { (host, $0) } }
        active = mine.map(\.1)

        // Which runners may still contribute to `fleet.json`.
        //
        // Here rather than in `retire`, because this is the one place that sees
        // the whole live set — and what the merge needs is the set, not the
        // event. A runner nobody is polling with its agents on a lock screen,
        // forever, is the bug merging creates if this is missing; see
        // `FleetPublication.keeping(runners:)`.
        FleetSnapshotWriter.keep(runners: Set(mine.map { $0.0.id.uuidString }))

        entries = mine.flatMap { host, connection -> [FleetEntry] in
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
    /// By runner id and never by workspace id, for `FleetEntry.id`'s reason: a
    /// workspace id says nothing about which runner it is on, so answering from
    /// one means searching every connection and taking the first match.
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

    // MARK: - What the screens ask

    /// Whether ANY runner has said what it has, at least once.
    ///
    /// `Connection.hasFleet`'s question asked of a fleet rather than of a
    /// runner, and the difference is the whole of why the app no longer blanks
    /// for one sleeping laptop: the shell opens as soon as there is a pane to
    /// open on, wherever it is, and a runner still connecting is a row rather
    /// than a screen. See `RunnerStatusRow`.
    var hasFleet: Bool { active.contains { $0.hasFleet } }

    /// The runners being talked to, paired with what they are, in list order.
    ///
    /// For the screens that draw a row per runner. `active` alone cannot answer
    /// it — a `Connection` reports an id and this is what turns that back into
    /// the runner a person named.
    var runners: [(host: Runner, connection: Connection)] {
        // `runnerOrder` and not `hosts.hosts`, so this and `publish` give one
        // answer to "which runners does this store have". They disagreed in the
        // layout harness, where the store stands on a canned connection and its
        // `RunnerStore` is empty: `publish` built entries and this reported
        // none, so the merged fleet on screen and the list of runners over it
        // were describing two different fleets.
        runnerOrder.compactMap { host in connections[host.id].map { (host, $0) } }
    }

    #if DEBUG
    /// Stand this store on one connection nobody dialed, for
    /// `AgentLayoutHarness` — the same trick `Connection.standIn(on:)` plays,
    /// one layer up, and for the same reason.
    ///
    /// The harness mounts the shipping shell over a canned fleet, and the shell
    /// reads a store now rather than a connection. Without this the harness
    /// would need a real `Runner` in a real `RunnerStore`, which is a harness
    /// that dials a machine.
    static func standIn(on connection: Connection, host: Runner) -> FleetStore {
        FleetStore(standingOn: connection, host: host)
    }

    /// **Dials nothing, ever**, which is the whole difference from the ordinary
    /// initializer and the reason it is not one.
    ///
    /// Going through `init(hosts:scope:)` would reconcile against a fresh
    /// `RunnerStore` — and a `RunnerStore` is not empty in the simulator: the
    /// UI suite launches with a demo runner as an argument, `init` picks it up,
    /// and the harness would open an SSH session to it before drawing a single
    /// canned pane. A layout harness that connects to a machine is not a
    /// fixture.
    private init(standingOn connection: Connection, host: Runner) {
        self.hosts = RunnerStore()
        self.everyRunnerAtOnce = false
        self.standInOrder = [host]
        self.connections[host.id] = connection
        self.dialed[host.id] = host
        // Watched, so filling the canned fleet in reaches the merge the same
        // way a poll does. No runner-list or settings observer: neither has
        // anything to say to a store with one connection nobody dialed.
        watchers[host.id] = connection.objectWillChange.sink { [weak self] _ in
            Task { @MainActor in self?.publish() }
        }
        publish()
    }

    /// The runner order to publish in when there is no `RunnerStore` behind
    /// this one. Empty in the app, where `hosts` is the answer.
    private var standInOrder: [Runner] = []

    /// Publish again, for a harness that has just filled its canned fleet in.
    ///
    /// `publish` runs off `Connection.objectWillChange`, which fires on the
    /// turn BEFORE the value lands — so a fixture written in a `.task` reaches
    /// the merge one body pass later, and the shell draws an empty fleet in
    /// between. The app never sees that gap because its first fleet arrives
    /// after several polls of nothing; a harness's arrives at once.
    func republish() { publish() }
    #endif
}
