import Foundation

/// Where a `Destination` actually opens, given what this device holds now
/// (ov-182, ov-183): the Mac's `TaskNoticeOpen.step`, generalized from a task
/// notice to every place, and from a click to a relaunch.
///
/// Pure, and called again on every change until it stops answering `wait`:
///
/// 1. **Find the runner.** By `host` first; by `id` among ready runners
///    next, waiting while any runner hasn't said its id, since that one
///    could be it. A destination naming no runner (an old agent push, a
///    decision from a runner too old to say) is looked for on every ready
///    runner, and refused when two have it.
/// 2. **Walk the place.** Each level is here, gone or not known yet. Not
///    known waits. Gone falls back to the next level up, quietly:
///    terminal → worktree → workspace → the runner's home (its first
///    workspace). Finer parts (tab, pane, agent, question) are dropped one
///    at a time when stale, and all together on a fall back.
/// 3. **At the deadline**, what's still unknown depends on how it arrived.
///    A restore opens the deepest level known to be there, then the last
///    workspace, then the first, then Needs You. A notification stays where
///    the click left the app, with a quiet note saying why.
/// 4. **Moving first wins.** A restore that somebody moved before, or that
///    a link landed on, does nothing.
///
/// A notification never falls back past its workspace: a pane that's gone
/// opening some other workspace on its runner is a surprise, not a help.
/// Mirrored case for case by Android's `DestinationResolver`, and both are
/// held to `test/fixtures/destinations.json`.
public enum DestinationResolver {
    /// How a destination came to be opened.
    public enum Arrival: String, Sendable {
        /// The last run's place, on a relaunch.
        case restore
        /// A tapped notification, or a link.
        case notification
    }

    /// How long each arrival waits for its runner and its place.
    public enum Deadline {
        /// `PhoneLaunch.decideWithin` and Android's `LaunchRule.WINDOW_MS`.
        public static let restore: TimeInterval = 10
        /// `TaskNoticeOpen.waitsAtMost`: a runner the Mac reaches answers in
        /// seconds.
        public static let notificationMac: TimeInterval = 30
        /// `PhoneDecisionLink.followWithin`: a phone's cold launch can take
        /// most of a minute to reach a runner.
        public static let notificationPhone: TimeInterval = 60
    }

    /// Why a notification left the app where it was.
    public enum Note: String, Sendable {
        /// Its runner never came up in time, or isn't one this device has.
        case runnerUnavailable = "runner-unavailable"
        /// Its runner answered, and the place wasn't found in time.
        case notFound = "not-found"
        /// Its runner says the place, and everything it knows above it, is gone.
        case gone
        /// It names no runner, and two runners have such a place.
        case ambiguous
    }

    public enum Resolution: Equatable, Sendable {
        /// Not yet: ask again when something changes.
        case wait
        /// Open this. Its runner carries the seat's `host` and `id`; a task
        /// carries its id and workspace; a terminal is resolved to its
        /// worktree (or its workspace's orchestrator) with `pane` set.
        /// `fellBack` when it isn't the level asked for.
        case open(Destination, fellBack: Bool)
        /// Do nothing. A notification says why; a restore somebody moved
        /// past says nothing.
        case stay(Note?)
    }

    /// Decide.
    ///
    /// - Parameters:
    ///   - elapsed: seconds since the click, or since the launch began.
    ///   - deadline: one of `Deadline`'s.
    ///   - interrupted: somebody moved, or a link is landing. Only a restore
    ///     yields to it.
    public static func resolve(
        _ destination: Destination, arrival: Arrival, in world: World, elapsed: TimeInterval,
        deadline: TimeInterval, interrupted: Bool = false
    ) -> Resolution {
        if arrival == .restore && interrupted { return .stay(nil) }
        let late = elapsed >= deadline
        if case .needsYou = destination.place { return .open(Destination.needsYou, fellBack: false) }

        switch findSeat(destination, in: world) {
        case .waiting:
            if !late { return .wait }
            return arrival == .restore ? general(world) : .stay(.runnerUnavailable)
        case .absent:
            return arrival == .restore ? general(world) : .stay(.runnerUnavailable)
        case .ambiguous:
            return arrival == .restore ? general(world) : .stay(.ambiguous)
        case .search:
            return search(destination, arrival: arrival, in: world, late: late)
        case .found(let seat):
            return walk(destination, on: seat, arrival: arrival, in: world, late: late)
        }
    }

    // MARK: - The runner

    private enum SeatSearch {
        case found(World.Seat)
        case waiting
        case absent
        case ambiguous
        /// The destination names no runner: look on every one.
        case search
    }

    private static func findSeat(_ destination: Destination, in world: World) -> SeatSearch {
        let runner = destination.runner
        if let host = runner.host, let seat = world.seats.first(where: { $0.host == host }) {
            return seat.ready ? .found(seat) : .waiting
        }
        guard let id = runner.id?.lowercased() else {
            return runner.host == nil ? .search : .absent
        }
        let matching = world.seats.filter { $0.ready && $0.runnerId?.lowercased() == id }
        if matching.count == 1 { return .found(matching[0]) }
        if matching.count > 1 { return .ambiguous }
        return world.seats.contains(where: { !$0.ready }) ? .waiting : .absent
    }

    /// A destination naming no runner: the one ready runner where its place
    /// is here. Two is refused, as `PhoneDecisionLink.find` refuses.
    private static func search(
        _ destination: Destination, arrival: Arrival, in world: World, late: Bool
    ) -> Resolution {
        var found: [Resolution] = []
        var unknown = world.seats.contains(where: { !$0.ready })
        for seat in world.seats where seat.ready {
            switch presence(destination.place, on: seat) {
            case .here: found.append(walk(destination, on: seat, arrival: arrival, in: world, late: late))
            case .unknown: unknown = true
            case .gone: break
            }
        }
        if found.count == 1 { return found[0] }
        if found.count > 1 { return arrival == .restore ? general(world) : .stay(.ambiguous) }
        if unknown && !late { return .wait }
        if arrival == .restore { return general(world) }
        return .stay(unknown ? .notFound : .gone)
    }

    // MARK: - The place

    enum Presence: Equatable {
        case here
        case gone
        case unknown
    }

    /// Whether `place` is on `seat`, without looking at its parents.
    static func presence(_ place: Destination.Place, on seat: World.Seat) -> Presence {
        switch place {
        case .needsYou:
            return .here
        case .workspace(let id):
            guard let workspaces = seat.workspaces else { return .unknown }
            return workspaces.contains(where: { $0.id == id }) ? .here : .gone
        case .orchestrator(let id):
            guard let workspaces = seat.workspaces else { return .unknown }
            return workspaces.contains(where: { $0.id == id && $0.orchestrator }) ? .here : .gone
        case .history(let id, _):
            return presence(.workspace(id), on: seat)
        case .task(let workspace, let ref):
            return seat.task(ref, in: workspace) == nil ? taskAbsence(ref, in: workspace, on: seat) : .here
        case .worktree(let id, _):
            guard let worktrees = seat.worktrees else { return .unknown }
            return worktrees.contains(where: { $0.id == id }) ? .here : .gone
        case .terminal(let id):
            guard let worktrees = seat.worktrees else { return .unknown }
            return worktrees.contains(where: { $0.terminals.contains(where: { $0.id == id }) }) ? .here : .gone
        }
    }

    /// A task not found: gone when its runner said so, or when every board
    /// it could be on has been read; not known yet otherwise.
    private static func taskAbsence(_ ref: Destination.TaskRef, in workspace: String?, on seat: World.Seat) -> Presence {
        if let id = ref.id, seat.absentTasks.contains(id) { return .gone }
        if let key = ref.key, seat.absentTasks.contains(key) { return .gone }
        if let workspace {
            // A task on a workspace that's gone is gone with it.
            if presence(.workspace(workspace), on: seat) == .gone { return .gone }
            return seat.boards[workspace] == nil ? .unknown : .gone
        }
        guard let workspaces = seat.workspaces else { return .unknown }
        return workspaces.allSatisfy({ seat.boards[$0.id] != nil }) ? .gone : .unknown
    }

    private static func walk(
        _ destination: Destination, on seat: World.Seat, arrival: Arrival, in world: World, late: Bool
    ) -> Resolution {
        let runner = Destination.Runner(host: seat.host, id: seat.runnerId)
        // The place asked for, then its parents, nearest first.
        let ladder = [destination.place] + destination.place.ancestors
        for (depth, place) in ladder.enumerated() {
            switch presence(place, on: seat) {
            case .gone:
                continue
            case .unknown:
                // Not known yet: wait for it, unless it's too late to.
                if !late { return .wait }
                if arrival == .notification { return .stay(.notFound) }
                continue
            case .here:
                if depth == 0 {
                    return .open(refine(destination, on: seat, runner: runner), fellBack: false)
                }
                var parent = Destination(runner: runner, place: place)
                if case .workspace = place, let segment = destination.segment, keeps(segment, place, on: seat) {
                    parent.segment = segment
                }
                return .open(parent, fellBack: true)
            }
        }
        // Nothing the destination names is here.
        if arrival == .notification { return .stay(.gone) }
        if let home = seat.workspaces?.first {
            return .open(Destination(runner: runner, place: .workspace(home.id)), fellBack: true)
        }
        return general(world)
    }

    /// The place asked for, which is here: resolved to ids, and each finer
    /// part kept only while it still applies.
    private static func refine(_ destination: Destination, on seat: World.Seat, runner: Destination.Runner) -> Destination {
        var out = destination
        out.runner = runner
        switch destination.place {
        case .task(let workspace, let ref):
            if let row = seat.task(ref, in: workspace) {
                out.place = .task(
                    workspace: row.workspace,
                    task: Destination.TaskRef(id: row.id, key: row.key ?? ref.key, repository: row.repository ?? ref.repository))
                if let tab = out.tab, let tabs = row.tabs, !tabs.contains(tab.rawValue) { out.tab = nil }
            }
        case .terminal(let id):
            if let found = seat.terminal(id) {
                let (worktree, terminal) = found
                if terminal.orchestrator, let workspace = worktree.workspace {
                    out.place = .orchestrator(workspace: workspace)
                } else {
                    out.place = .worktree(worktree.id, workspace: worktree.workspace)
                }
                out.pane = id
            }
        default:
            break
        }
        if case .task = out.place {} else {
            out.tab = nil
            out.agent = nil
            out.question = false
        }
        if case .workspace = out.place {
            if let segment = out.segment, !keeps(segment, out.place, on: seat) { out.segment = nil }
        } else {
            out.segment = nil
        }
        // Not read yet is kept: the platform checks it again as it lands.
        if seat.worktrees != nil {
            if let pane = out.pane, seat.terminal(pane) == nil { out.pane = nil }
            if let agent = out.agent, seat.terminal(agent) == nil { out.agent = nil }
        }
        return out
    }

    /// Whether a workspace screen can show `segment`: an orchestrator only
    /// on a workspace that has one.
    private static func keeps(_ segment: Destination.Segment, _ place: Destination.Place, on seat: World.Seat) -> Bool {
        guard segment == .orchestrator, case .workspace(let id) = place else { return true }
        return seat.workspaces?.contains(where: { $0.id == id && $0.orchestrator }) ?? false
    }

    /// Where a restore opens when its own runner has nothing for it: the
    /// last workspace, then the first workspace of the first ready runner,
    /// then Needs You.
    private static func general(_ world: World) -> Resolution {
        if let last = world.lastWorkspace,
            let seat = world.seats.first(where: { $0.ready && $0.host == last.host }),
            presence(.workspace(last.workspace), on: seat) == .here
        {
            return .open(
                Destination(runner: .init(host: seat.host, id: seat.runnerId), place: .workspace(last.workspace)),
                fellBack: true)
        }
        for seat in world.seats where seat.ready {
            if let first = seat.workspaces?.first {
                return .open(
                    Destination(runner: .init(host: seat.host, id: seat.runnerId), place: .workspace(first.id)),
                    fellBack: true)
            }
        }
        return .open(Destination.needsYou, fellBack: true)
    }
}

extension DestinationResolver {
    /// What this device holds now, as the resolver reads it. Each platform
    /// builds one from its own stores; `nil` collections are not read yet,
    /// which is different from read and empty.
    public struct World: Codable, Equatable, Sendable {
        public var seats: [Seat]
        /// The last workspace somebody was in (`workspace.last` on the
        /// phones), a restore's rung above the first workspace.
        public var lastWorkspace: Last?

        public init(seats: [Seat], lastWorkspace: Last? = nil) {
            self.seats = seats
            self.lastWorkspace = lastWorkspace
        }

        public struct Last: Codable, Equatable, Sendable {
            public var host: String
            public var workspace: String

            public init(host: String, workspace: String) {
                self.host = host
                self.workspace = workspace
            }
        }

        /// One runner this device holds.
        public struct Seat: Codable, Equatable, Sendable {
            /// The device's own handle: `DaemonClient.target`, `Host.id`.
            public var host: String
            /// Its `Host.runner_id`, once read.
            public var runnerId: String?
            /// Connected, and its status read: `runnerId` is what it says.
            public var ready: Bool
            /// Its workspaces, in the switcher's order. Nil until read.
            public var workspaces: [Workspace]?
            /// Its worktrees and their panes. Nil until its fleet is read.
            public var worktrees: [Worktree]?
            /// Its boards read so far, by workspace id. A workspace with no
            /// entry hasn't been read.
            public var boards: [String: [Task]]
            /// Task ids or keys a direct lookup (`task show`) said this
            /// runner doesn't have.
            public var absentTasks: [String]

            public init(
                host: String, runnerId: String? = nil, ready: Bool, workspaces: [Workspace]? = nil,
                worktrees: [Worktree]? = nil, boards: [String: [Task]] = [:], absentTasks: [String] = []
            ) {
                self.host = host
                self.runnerId = runnerId
                self.ready = ready
                self.workspaces = workspaces
                self.worktrees = worktrees
                self.boards = boards
                self.absentTasks = absentTasks
            }

            public init(from decoder: Decoder) throws {
                let c = try decoder.container(keyedBy: CodingKeys.self)
                self.init(
                    host: try c.decode(String.self, forKey: .host),
                    runnerId: try c.decodeIfPresent(String.self, forKey: .runnerId),
                    ready: try c.decode(Bool.self, forKey: .ready),
                    workspaces: try c.decodeIfPresent([Workspace].self, forKey: .workspaces),
                    worktrees: try c.decodeIfPresent([Worktree].self, forKey: .worktrees),
                    boards: try c.decodeIfPresent([String: [Task]].self, forKey: .boards) ?? [:],
                    absentTasks: try c.decodeIfPresent([String].self, forKey: .absentTasks) ?? [])
            }
        }

        public struct Workspace: Codable, Equatable, Sendable {
            public var id: String
            /// Whether it has an orchestrator. An implicit workspace, on a
            /// runner without workspaces, never does.
            public var orchestrator: Bool

            public init(id: String, orchestrator: Bool = true) {
                self.id = id
                self.orchestrator = orchestrator
            }

            public init(from decoder: Decoder) throws {
                let c = try decoder.container(keyedBy: CodingKeys.self)
                self.init(
                    id: try c.decode(String.self, forKey: .id),
                    orchestrator: try c.decodeIfPresent(Bool.self, forKey: .orchestrator) ?? true)
            }
        }

        public struct Worktree: Codable, Equatable, Sendable {
            public var id: String
            /// Its workspace; nil for a loose one.
            public var workspace: String?
            public var terminals: [Terminal]

            public init(id: String, workspace: String? = nil, terminals: [Terminal] = []) {
                self.id = id
                self.workspace = workspace
                self.terminals = terminals
            }
        }

        public struct Terminal: Codable, Equatable, Sendable {
            public var id: String
            /// Its workspace's orchestrator pane.
            public var orchestrator: Bool

            public init(id: String, orchestrator: Bool = false) {
                self.id = id
                self.orchestrator = orchestrator
            }

            public init(from decoder: Decoder) throws {
                let c = try decoder.container(keyedBy: CodingKeys.self)
                self.init(
                    id: try c.decode(String.self, forKey: .id),
                    orchestrator: try c.decodeIfPresent(Bool.self, forKey: .orchestrator) ?? false)
            }
        }

        /// One task row on a board, or a `task show` result.
        public struct Task: Codable, Equatable, Sendable {
            public var id: String
            public var key: String?
            public var repository: String?
            /// The tabs its task view offers; nil is every tab.
            public var tabs: [String]?

            public init(id: String, key: String? = nil, repository: String? = nil, tabs: [String]? = nil) {
                self.id = id
                self.key = key
                self.repository = repository
                self.tabs = tabs
            }
        }
    }
}

extension DestinationResolver.World.Seat {
    /// A task row matching `ref`, on `workspace`'s board or, with none
    /// named, on any board, with the workspace it was found on. By id when
    /// the ref has one, else by key, in the ref's repository when it says.
    func task(_ ref: Destination.TaskRef, in workspace: String?) -> (id: String, key: String?, repository: String?, tabs: [String]?, workspace: String)? {
        let names = workspace.map { [$0] } ?? (workspaces?.map(\.id) ?? boards.keys.sorted())
        for name in names {
            for row in boards[name] ?? [] where matches(row, ref) {
                return (row.id, row.key, row.repository, row.tabs, name)
            }
        }
        return nil
    }

    private func matches(_ row: DestinationResolver.World.Task, _ ref: Destination.TaskRef) -> Bool {
        if let id = ref.id { return row.id == id }
        guard let key = ref.key, row.key == key else { return false }
        guard let repository = ref.repository, let own = row.repository else { return true }
        return own.lowercased() == repository.lowercased()
    }

    /// The worktree holding terminal `id`, and the terminal.
    func terminal(_ id: String) -> (DestinationResolver.World.Worktree, DestinationResolver.World.Terminal)? {
        for worktree in worktrees ?? [] {
            if let terminal = worktree.terminals.first(where: { $0.id == id }) { return (worktree, terminal) }
        }
        return nil
    }
}
