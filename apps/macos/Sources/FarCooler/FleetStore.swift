import AgentKit
import Combine
import SwiftUI

/// Every runner, at once.
///
/// `Fleet` is one runner's decoded `worktree list --json`; this holds N of
/// them and publishes the merge. Views observe this one object rather than
/// subscribing to a client per runner.
///
/// Membership is always the local runner plus one client per configured runner.
/// `Runners.all` lists remote runners only — this Mac is the implicit entry
/// under the empty-string key, which is the same convention `worktree.host`
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

    /// The app's one store, which every window shares (ov-133).
    ///
    /// Each window used to build its own, so two windows meant two clients
    /// per runner, two event streams, two copies of every fleet, and the
    /// newest window taking `Reachability`'s single wake and retry hook from
    /// the others. What a window chooses (its selection, its panes) stays the
    /// window's; what the runners say is the app's, once.
    static let shared = FleetStore()

    /// Whether this store dials its runners: the app's does, from the first
    /// window that opens; a test's (`init(clients:)`) never does.
    private let dials: Bool
    /// Whether a window has opened yet. Nothing is dialed before one has, so
    /// a store that's only been named, by a test or a Settings pane, starts
    /// no process.
    private var started = false
    /// The windows open on this store. See `WindowSet`.
    private(set) var windows = WindowSet()

    init() {
        dials = true
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

    /// A store over `clients`, for a test: it dials nothing, follows no
    /// runner list, and leaves `Reachability`'s hook to the app's store.
    init(clients: [String: DaemonClient]) {
        dials = false
        for (target, client) in clients {
            self.clients[target] = client
            clientObservers[target] = client.objectWillChange.sink { [weak self] _ in
                Task { @MainActor in self?.scheduleRemerge() }
            }
        }
        remerge()
    }

    /// A window opened on this store: the first one brings every runner up,
    /// and any window brings back an event stream the last one to close
    /// stopped.
    ///
    /// The first window leaves each stream to its runner's bring-up, which
    /// starts it after `daemon ensure` and the first read, as each window
    /// did before the store was shared (ov-296). Resuming here too started
    /// every stream ahead of `ensure`: on a launch that starts the daemon,
    /// the local stream failed against a socket not yet there and waited
    /// out a retry.
    func open(window: UUID) {
        windows.open(window)
        guard dials else { return }
        guard started else {
            started = true
            for target in clients.keys where bringUpTasks[target] == nil { bringUp(target) }
            return
        }
        resume()
    }

    /// A window closed. Only the last one stops the streams: the others are
    /// still showing what they bring.
    func close(window: UUID) {
        guard windows.close(window) else { return }
        for client in clients.values { client.stopEvents() }
    }

    /// Every runner, local first.
    var hosts: [String] { [""] + Runners.shared.all.map(\.target) }

    /// Resume every client's event stream, as each window opens.
    ///
    /// A fresh client's stream starts once, at its bring-up. The last window
    /// to close stops every stream (`close(window:)`), and the next window to
    /// open (⌘W, then the Dock) needs them back: without this it came back
    /// with `state` still reading `.connected` and a healthy-looking bar, but
    /// no stream, no retry and no timer underneath it. `startEvents()`'s own
    /// `eventStream == nil` guard makes this safe to call unconditionally.
    private func resume() {
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
                Task { @MainActor in self?.scheduleRemerge() }
            }
            // Not before a window has opened: see `started`.
            if started { bringUp(target) }
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

    /// Bring a freshly-added client up: its first `refresh()`, then its
    /// event stream. See `bringUpTasks`.
    private func bringUp(_ target: String) {
        guard let client = clients[target] else { return }
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

    /// What the status bar may say about the fleet's panes.
    ///
    /// Three answers, not two, because "how many panes are live" has a third
    /// honest reply: this app can't say. Only a runner that is `.connected`
    /// right now has a count worth adding up; one that is connecting,
    /// reconnecting, unreachable or not installed has only the last fleet it
    /// read before the link went, and the agents in it may have exited since.
    /// Reporting that as live is hands on deck that went home; reporting it
    /// as "tmux unavailable" in red is a death nobody saw. So a fleet with no
    /// runner connected says neither.
    enum Reading: Equatable {
        /// At least one connected runner has tmux: this many panes, across
        /// the connected runners only.
        case live(Int)
        /// Runners are connected, and not one of them can reach tmux.
        case runtimeDown
        /// Nothing is known yet: every runner is still on its first read.
        case connecting
        /// No runner is connected, and at least one has been lost or
        /// refused. Neither a count nor a failure — see above.
        case unsaid
        /// No runner is connected, and every one that has answered at all
        /// said Far Cooler isn't installed there: the runners, by name, "" for
        /// this Mac. Said, because it is the one reason here with something
        /// to do about it, and "Not connected" would hide it — on a fleet of
        /// one the trouble dots that would name it are hidden.
        case notInstalled([String])

        /// The words beside the dot.
        var sentence: String {
            switch self {
            case .live(let count): return "\(count) live"
            case .runtimeDown: return "tmux unavailable"
            case .connecting: return "Connecting…"
            case .unsaid: return "Not connected"
            case .notInstalled(let runners) where runners.count == 1:
                let runner = runners[0].isEmpty ? "this Mac" : runners[0]
                return "Far Cooler isn’t installed on \(runner)"
            case .notInstalled(let runners):
                return "Far Cooler isn’t installed on \(runners.count) runners"
            }
        }

        /// Red only for the one reading that is a failure someone saw.
        var isTrouble: Bool { self == .runtimeDown }
    }

    /// The reading from each runner's state and the last fleet it read.
    ///
    /// Gated on `.connected`, not on `state.refusal == nil`, which is what this
    /// used to test. A runner that stays down spends most of its outage in
    /// `.reconnecting`, not `.unreachable`: each retry's `refresh()` fails and
    /// sets `.unreachable`, then the event stream that follows it dies and its
    /// `onEnd` resets the state to `.reconnecting` for the whole backoff wait
    /// — up to 30 s. `refusal` is nil there, so the dead runner's last count
    /// and its last `runtimeHealthy` came back for every one of those waits.
    /// `BoardAgents.on` fixed the same thing for the board's pills the same
    /// way.
    static func reading(of runners: [(host: String, state: HostState, fleet: Fleet)]) -> Reading {
        let connected = runners.filter { $0.state == .connected }.map(\.fleet)
        guard connected.isEmpty else {
            // Healthy is an OR across the connected runners, and the count is
            // every connected runner's, healthy or not — see `unhealthyHosts`
            // for why the OR stays an OR, and for how the bar names a runner
            // it leaves out.
            guard connected.contains(where: \.runtimeHealthy) else { return .runtimeDown }
            return .live(connected.reduce(0) { $0 + $1.livePanes })
        }
        let answered = runners.filter { $0.state != .connecting }
        if answered.isEmpty { return .connecting }
        // Neutral like `.unsaid`, not red: a runner without Far Cooler has
        // lost nothing, and the trouble dot paints it `.secondary` too.
        if answered.allSatisfy({ $0.state == .notInstalled }) {
            return .notInstalled(answered.map(\.host))
        }
        return .unsaid
    }

    /// See `Reading`.
    @Published private(set) var reading: Reading = .connecting

    /// One re-merge for however many client changes arrive together.
    ///
    /// A client fires `objectWillChange` once per `@Published` write, and one
    /// daemon event used to be thirteen of them, each re-merging every runner
    /// and republishing the whole window (ov-229). The writes of one turn now
    /// share one pass, which runs after they have all landed.
    ///
    /// The pass still says the store changed even when the merged fleet did
    /// not, because views read client state through this store — a layout, a
    /// changes row, a needs-you count — and this forwarding is how they hear
    /// about it. What no longer happens is saying it thirteen times.
    private func scheduleRemerge() {
        guard !remergeScheduled else { return }
        remergeScheduled = true
        Task { @MainActor [weak self] in
            guard let self else { return }
            self.remergeScheduled = false
            self.remerge(forwarding: true)
        }
    }

    private var remergeScheduled = false

    /// One list from N.
    ///
    /// An unreachable runner still contributes its last good rows — that is
    /// what keeps your mental map of the fleet stable while a laptop sleeps. A
    /// runner that has never connected contributes none, and appears only as a
    /// header with its state.
    ///
    /// Its panes are not counted, though, and its tmux health does not speak
    /// for the fleet: see `reading(of:)`. `fleet.livePanes` and
    /// `fleet.runtimeHealthy` here are the connected runners' only, so that
    /// nothing reading them can take a stale `true` for a live one. Without
    /// that, a single lost runner with a stale `runtimeHealthy == true` could
    /// keep the whole bar reading well while the only runner actually
    /// answering has no tmux at all.
    private func remerge(forwarding: Bool = false) {
        let merged = Self.merge(
            hosts.compactMap { host in clients[host].map { (host, $0.state, $0.fleet) } })
        // Assigned only when it changed: `@Published` fires on every
        // assignment, and this property is what the root view observes.
        if fleet != merged.fleet {
            fleet = merged.fleet
        } else if forwarding {
            objectWillChange.send()
        }
        if reading != merged.reading { reading = merged.reading }
        let items = Self.shownNeedsYou(clients)
        if items != needsYou { needsYou = items }
        let unanswered = Self.unansweredNames(hosts, clients)
        if unanswered != needsYouUnanswered { needsYouUnanswered = unanswered }
        let settled = Self.settled(clients.values.map { ($0.state, $0.needsYouKnown) })
        if settled != needsYouSettled { needsYouSettled = settled }
    }

    /// What Needs You shows: each runner's own list where it has been read, and
    /// for every other runner the blocked agents its fleet shows
    /// (`PhoneInbox.shown`, the rule the phones and the glances use).
    static func shownNeedsYou(_ clients: [String: DaemonClient]) -> [NeedsYouItem] {
        var lists: [String: [NeedsYouItem]] = [:]
        var unread: [String: [NeedsYou.OlderPane]] = [:]
        for (host, client) in clients {
            if let list = client.needsYouList {
                lists[host] = list
            } else {
                unread[host] = DaemonClient.olderPanes(in: client.fleet.worktrees)
            }
        }
        return PhoneInbox.shown(lists: lists, unread: unread)
    }

    /// What the caveat calls this Mac's own runner. The runner is the word, and
    /// it opens the sentence, so it is capitalized: "This Mac’s runner isn’t
    /// answering, so this may not be everything."
    static let localRunnerName = "This Mac’s runner"

    /// The runners that haven't answered, by the name the caveat uses.
    static func unansweredNames(_ hosts: [String], _ clients: [String: DaemonClient]) -> [String] {
        hosts.filter { clients[$0]?.needsYouUnanswered == true }
            .map { $0.isEmpty ? localRunnerName : $0 }
    }

    /// The runners that haven't said what needs a person, by name, for the
    /// caveat under the list (`PhoneInbox.caveat`).
    @Published private(set) var needsYouUnanswered: [String] = []

    /// Whether every runner has said what's waiting, as far as it can: none
    /// still making its first connection, and every one that's connected
    /// has read its list. What the window waits for before opening anywhere
    /// but Needs You, so it doesn't open on a workspace a moment before a
    /// runner's decisions arrive.
    @Published private(set) var needsYouSettled = false

    static func settled(_ runners: [(state: HostState, known: Bool)]) -> Bool {
        runners.allSatisfy { runner in
            switch runner.state {
            case .connecting: return false
            case .connected: return runner.known
            default: return true
            }
        }
    }

    /// Everything a person has to act on, on every runner, most urgent first:
    /// each runner's own list, or its derived one, merged by rank
    /// (`NeedsYou.merge`). Each item's `runner` is its host, `""` for this
    /// Mac. What the Needs You row counts and ⌃⌘N walks.
    @Published private(set) var needsYou: [NeedsYouItem] = []

    /// `remerge`'s arithmetic, apart from the clients it reads, so it can be
    /// asked about: every runner's rows in order, and the connected runners'
    /// count and health only.
    static func merge(_ runners: [(host: String, state: HostState, fleet: Fleet)])
        -> (fleet: Fleet, reading: Reading)
    {
        let reading = reading(of: runners)
        let healthy: Bool, live: Int
        switch reading {
        case .live(let count): (healthy, live) = (true, count)
        case .runtimeDown, .connecting, .unsaid, .notInstalled: (healthy, live) = (false, 0)
        }
        var fleet = Fleet(
            runtimeHealthy: healthy, livePanes: live,
            worktrees: runners.flatMap(\.fleet.worktrees))
        // Each runner's workspaces, under that runner. Kept from an
        // unreachable runner as its rows are, so its sidebar stays put.
        for runner in runners {
            fleet.runnerWorkspaces.merge(runner.fleet.runnerWorkspaces) { first, _ in first }
        }
        return (fleet, reading)
    }

    /// Runners that are not fully healthy right now, for the status bar to
    /// name individually.
    ///
    /// `fleet.runtimeHealthy` above ORs across every runner, and stays an OR
    /// on purpose — ANDing would turn the whole status bar red every time any
    /// one laptop was merely asleep, which is not news worth a color change.
    /// But a single merged boolean is also the whole story only if nobody
    /// needs to know WHICH runner is the problem, and with more than one
    /// runner configured that is exactly the question the merged dot cannot
    /// answer: it takes only one healthy runner to keep it quiet while a
    /// second sits there with no tmux at all. This is how the bar can name
    /// that second runner instead of just going quiet about it.
    ///
    /// Two colors in that paragraph have moved out from under it, and both
    /// arguments survive the move. The ANDed bar was "orange" until `8741757`
    /// made an unreadable tmux red rather than amber — nobody is waiting, the
    /// runtime is not answering. And the merged dot was "a green dot ... turn
    /// it green" until the healthy half went neutral: green in this palette is
    /// `Status.done`, and a dot that is always on is a dot nobody reads. The
    /// point was never the hue. It is that one boolean ORed across a fleet
    /// cannot name a runner, so something else has to.
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

    /// Runners running a daemon that is not this app's build, in `hosts`
    /// order — this Mac first.
    ///
    /// Deliberately NOT folded into `unhealthyHosts` above, which is the list
    /// the status bar's trouble dots come from. Three reasons, and all three
    /// would be bugs if this were merged into it:
    ///
    /// - A stale runner is not unhealthy. It answers every read, its tmux is
    ///   fine, and its panes are live; what is wrong is that it is a different
    ///   program from the one this app was built against. Painting it with the
    ///   same dot as a runner that has gone dark would make both mean less.
    /// - A trouble dot RECONNECTS when clicked, and a connection that is
    ///   already up has nothing to retry. The action here has to ask first —
    ///   it costs every agent conversation on that runner.
    /// - `unhealthyHosts` leaves out a runner still in `.connecting` because
    ///   nothing is known about it yet. This list is empty for exactly the
    ///   same runner and for a different reason: a version this client has not
    ///   read is not a version to make claims about.
    var staleHosts: [String] {
        hosts.filter { clients[$0]?.daemonSkew.offersUpdate == true }
    }

    /// Runners newer than this Mac (`DaemonSkew.ahead`, ov-143): never in
    /// `staleHosts`, since the only "update" from here would install this
    /// Mac's older build over them. The runner item says to update this Mac.
    var aheadHosts: [String] {
        hosts.filter { clients[$0]?.daemonSkew.isAhead == true }
    }

    // MARK: - Routing

    /// The runner a row came from.
    ///
    /// By `worktree.host`, which the CLI stamps from the `--host` flag it was
    /// invoked with. Never by id: short ids are the last eight hex of a UUID
    /// minted per daemon, so they say nothing about which runner they are on.
    func client(for worktree: Worktree) -> DaemonClient? {
        clients[worktree.host ?? ""]
    }

    func state(of host: String) -> HostState {
        clients[host]?.state ?? .connecting
    }

    // MARK: - Per-runner reads

    /// Repositories across every runner, each tagged with the runner it is on.
    ///
    /// Not merged into a flat `[Repository]`: a repository's own short id is
    /// eight hex characters minted per daemon, same as a worktree's, so two
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

    /// One worktree's layout is per-runner as well as per-worktree, so the
    /// key carries both — a flat `[String: [PaneGroup]]` across several
    /// runners could let one runner's layout answer for another's
    /// worktree of the same id.
    struct LayoutKey: Hashable {
        var host: String
        var worktree: String
    }

    /// Every runner's layouts, merged. Read through `client(for:)` when a
    /// worktree is already in hand — this exists for the one place that has
    /// to watch every runner's layouts at once regardless of which is
    /// selected: `ContentView`'s `.onChange(of:)` that keeps the app looking
    /// at wherever tmux just moved focus to.
    var layouts: [LayoutKey: [PaneGroup]] {
        var merged: [LayoutKey: [PaneGroup]] = [:]
        for host in hosts {
            guard let client = clients[host] else { continue }
            for (worktree, groups) in client.layouts {
                merged[LayoutKey(host: host, worktree: worktree)] = groups
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
    func refusal(for worktree: Worktree) -> String? {
        refusal(for: worktree.host ?? "")
    }

    /// Same check, from a bare host rather than a worktree already on it —
    /// for the handful of mutations (new task, resume branch, add root,
    /// register repository, new worktree from the sidebar's own `+`) that
    /// have no worktree in hand yet to route by.
    func refusal(for host: String) -> String? {
        guard let client = clients[host] else {
            return "this runner is no longer configured"
        }
        return client.state.refusal
    }

    /// The same check, as the sentence a banner shows: `ActionCopy.refused`.
    /// `refusal(for:)` is the runner's own words, for a sheet's `DetailBox`.
    func refusalSentence(for host: String) -> String? {
        guard refusal(for: host) != nil else { return nil }
        return ActionCopy.refused(clients[host]?.state)
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

/// The windows open on the app's one `FleetStore` (ov-133), by each window's
/// id: what decides when the runners' event streams may stop. A window
/// closing used to stop every stream on its way out, which was harmless only
/// while each window had streams of its own.
struct WindowSet: Equatable {
    private(set) var ids: Set<UUID> = []

    /// `window` opened.
    mutating func open(_ window: UUID) { ids.insert(window) }

    /// `window` closed: true when it was the last one open, and the streams
    /// can stop. A window that closes twice, or never opened, ends nothing.
    mutating func close(_ window: UUID) -> Bool {
        guard ids.remove(window) != nil else { return false }
        return ids.isEmpty
    }
}
