import AgentKit
import AppKit
import Foundation
import UserNotifications

/// A destination waiting to be opened (ov-106, ov-182, ov-183): what a
/// notification's click is about, or the place the last run was in.
///
/// A click names its subject by what a push carries, never by anything a
/// selection is made of: a task by key and its runner by `Host.runner_id`, a
/// pane by terminal id. A task's id and its workspace's id are read from the
/// runner (`task show`) once that runner is connected. A click can come
/// before that, on a cold launch from the notification or with the runner
/// still dialing, so the open waits for it, at most its arrival's deadline
/// (`DestinationResolver.Deadline`), and then leaves the app where the click
/// brought it: in front. A relaunch waits ten seconds, then opens the nearest
/// level of its place that is still there.
struct DestinationOpen: Equatable, Sendable {
    var destination: Destination
    var arrival: DestinationResolver.Arrival
    /// When it began, for the deadline.
    var since: Date
    /// Tells two clicks on the same notification apart, so the second is
    /// opened again rather than taken for the first.
    var id = UUID()

    init(destination: Destination, arrival: DestinationResolver.Arrival, since: Date) {
        self.destination = destination
        self.arrival = arrival
        self.since = since
    }

    /// The open a response asks for: a plain click, on a notification about
    /// something this build can open. Nil for an answer button, "Answer…" and
    /// a dismissal. `thread` is the notification's thread identifier, which
    /// the banners this app posts file under their terminal's id.
    init?(userInfo: [AnyHashable: Any], thread: String, action: String, now: Date) {
        guard action == UNNotificationDefaultActionIdentifier,
            let destination = Destination(userInfo: userInfo, thread: thread)
        else { return nil }
        self.init(destination: destination, arrival: .notification, since: now)
    }

    /// How long it waits for its runner and its place.
    var deadline: TimeInterval {
        arrival == .restore ? DestinationResolver.Deadline.restore : DestinationResolver.Deadline.notificationMac
    }

    /// Where `task show --json` says the task is: its id, and its board,
    /// which is its workspace or, on a runner without workspaces, its
    /// repository's implicit one, whose id is the repository's
    /// (`ContentView.Selection.workspace`).
    struct TaskShow: Equatable, Sendable {
        var task: String
        var workspace: String

        /// The memberwise init, spelled out: `init?(show:)` below would
        /// otherwise take it away.
        init(task: String, workspace: String) {
            self.task = task
            self.workspace = workspace
        }

        /// Nil for a read that names no task, or one under another key or,
        /// when `repository` is said, in another repository.
        init?(show data: Data, key: String, repository: String? = nil) {
            guard let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let task = body["task"] as? [String: Any],
                task["key"] as? String == key,
                repository == nil || (task["repository_id"] as? String)?.lowercased() == repository?.lowercased(),
                let id = task["id"] as? String, !id.isEmpty
            else { return nil }
            let workspace = (task["workspace"] as? String).flatMap { $0.isEmpty ? nil : $0 }
            guard let board = workspace ?? (task["repository_id"] as? String), !board.isEmpty else { return nil }
            self.init(task: id, workspace: board)
        }
    }
}

/// What one open has read from its runners, merged into the world it's
/// resolved against: a task by key (`task show`) or a workspace's board, so a
/// task the fleet doesn't list is found, or known to be gone.
struct DestinationReads: Equatable, Sendable {
    typealias World = DestinationResolver.World

    enum Request: Hashable, Sendable {
        /// The task under this key, in the repository when it's said.
        case task(key: String, repository: String?)
        /// One workspace's board, by id.
        case board(workspace: String)
    }

    private var asked: Set<String> = []
    private var boards: [String: [String: [World.Task]]] = [:]
    private var absent: [String: [String]] = [:]

    /// The reads `destination` still needs from the runners in `world`, none
    /// asked twice. A task by key is asked of every ready runner it could be
    /// on; a task by id, as a relaunch holds it, of its workspace's board.
    func owed(for destination: Destination, in world: World) -> [(host: String, request: Request)] {
        guard case .task(let workspace, let ref) = destination.place else { return [] }
        var owed: [(host: String, request: Request)] = []
        for seat in world.seats where seat.ready {
            if let host = destination.runner.host, host != seat.host { continue }
            if let id = destination.runner.id, seat.runnerId?.lowercased() != id.lowercased() { continue }
            let request: Request
            if let workspace, ref.id != nil {
                // Only a board the runner lists: a workspace that's gone has no board to read.
                guard seat.workspaces?.contains(where: { $0.id == workspace }) == true else { continue }
                request = .board(workspace: workspace)
            } else if let key = ref.key {
                request = .task(key: key, repository: ref.repository)
            } else {
                continue
            }
            if !asked.contains(Self.token(seat.host, request)) { owed.append((seat.host, request)) }
        }
        return owed
    }

    mutating func markAsked(_ host: String, _ request: Request) {
        asked.insert(Self.token(host, request))
    }

    /// What `host` answered. A read that failed records nothing: the open
    /// waits out its deadline rather than calling a task gone that a dropped
    /// link kept from being read.
    mutating func record(_ data: Data?, for request: Request, on host: String) {
        guard let data else { return }
        switch request {
        case .task(let key, let repository):
            if let found = DestinationOpen.TaskShow(show: data, key: key, repository: repository) {
                boards[host, default: [:]][found.workspace, default: []]
                    .append(World.Task(id: found.task, key: key, repository: repository))
            } else {
                absent[host, default: []].append(key)
            }
        case .board(let workspace):
            guard let board = try? TaskBoardModel.decode(data) else { return }
            boards[host, default: [:]][workspace] = board.rows.map { World.Task(id: $0.id, key: $0.key) }
        }
    }

    /// `world` with what's been read. A workspace a read found a task on is
    /// listed, if the runner's list didn't have it: its board's own say.
    func applied(to world: World) -> World {
        var out = world
        for index in out.seats.indices {
            let host = out.seats[index].host
            for (workspace, rows) in boards[host] ?? [:] {
                out.seats[index].boards[workspace, default: []] += rows
                if out.seats[index].workspaces?.contains(where: { $0.id == workspace }) == false {
                    out.seats[index].workspaces?.append(World.Workspace(id: workspace, orchestrator: false))
                }
            }
            out.seats[index].absentTasks += absent[host] ?? []
        }
        return out
    }

    private static func token(_ host: String, _ request: Request) -> String { "\(host)|\(request)" }

    /// What a runner is asked on an open's behalf: a task by its key, or one
    /// workspace's board, both as the board and the notice already read them.
    /// Nil for a runner this Mac doesn't hold, or a read that failed.
    @MainActor
    static func read(_ request: Request, from client: DaemonClient?, fleet: Fleet) async -> Data? {
        guard let client else { return nil }
        switch request {
        case .task(let key, let repository):
            return await client.taskByKey(key, repository: repository).data
        case .board(let workspace):
            let summary =
                fleet.runnerWorkspaces[client.target]?.first { $0.id == workspace }
                ?? WorkspaceSummary.implicit(repository: workspace)
            return await client.taskBoard(
                repository: summary.repository ?? summary.id, workspace: summary.boardWorkspace
            ).data
        }
    }
}

/// The destination waiting to be opened by a click, shared by every window:
/// the notification center's delegate is the app's, and which window opens it
/// is the windows' to settle (`claim`).
@MainActor
final class DestinationOpener: ObservableObject {
    static let shared = DestinationOpener()

    /// The click not yet opened or given up on.
    @Published private(set) var pending: DestinationOpen?
    /// The window that took `pending` on.
    private var claimedBy: UUID?

    /// How long a window that isn't key leaves a click to the key one.
    static let keyWindowFirst: TimeInterval = 0.5

    /// The main windows that are open. A click with none has nothing to open
    /// it, so it opens one.
    private var windows: Set<UUID> = []
    /// Opens a main window: the app's `openWindow`, set by `FarCoolerApp`.
    var openMainWindow: (() -> Void)?
    /// Brings the app forward. Replaced in tests.
    var activateApp: () -> Void = { NSApp.activate() }

    func register(window: UUID) { windows.insert(window) }
    func unregister(window: UUID) { windows.remove(window) }

    /// A click to open, in place of any before it. With no window open the app
    /// comes forward and opens one, which then takes the click (`drive`).
    func request(_ open: DestinationOpen) {
        pending = open
        claimedBy = nil
        guard windows.isEmpty else { return }
        activateApp()
        openMainWindow?()
    }

    /// Whether `window` opens `open`: the first to ask, except that a window
    /// that isn't key waits `keyWindowFirst` for the key one.
    func claim(_ open: DestinationOpen, window: UUID, isKey: Bool, now: Date) -> Bool {
        guard pending?.id == open.id else { return false }
        if let claimedBy { return claimedBy == window }
        guard isKey || now.timeIntervalSince(open.since) >= Self.keyWindowFirst else { return false }
        claimedBy = window
        return true
    }

    /// `open` is done with: opened, or given up on.
    func finish(_ open: DestinationOpen) {
        guard pending?.id == open.id else { return }
        pending = nil
        claimedBy = nil
    }

    /// How an open ended.
    enum Outcome: Equatable, Sendable {
        case opened
        /// Left the app where it was, and why.
        case stayed(DestinationResolver.Note?)
        /// Something else took its place, or the window went away.
        case cancelled
    }

    /// Resolve `open` until it opens or is dropped, asking again as the runners
    /// come up: `world` is read afresh on every pass, `read` asks a runner for
    /// what the fleet doesn't hold, and `land` is the window's own way of
    /// opening what resolved. `isCurrent` is false once something else has
    /// taken `open`'s place; `interrupted` is whether somebody moved first,
    /// which only a relaunch yields to. `pause` is the wait between passes.
    ///
    /// The deadline holds for a read too: a task that turns up after it opens
    /// nothing, for a click, since it would move a window somebody has gone on
    /// using.
    static func run(
        _ open: DestinationOpen, isCurrent: () -> Bool, interrupted: () -> Bool = { false },
        world: () -> DestinationResolver.World,
        read: (_ host: String, _ request: DestinationReads.Request) async -> Data?,
        land: (Destination) -> Void,
        now: () -> Date = { Date() },
        pause: () async -> Void = { try? await Task.sleep(for: .milliseconds(250)) }
    ) async -> Outcome {
        var reads = DestinationReads()
        // When the deadline's clock started: the open's own start, or for a
        // restore whose runner is still coming, the moment it came (ov-279).
        var since = open.since
        while !Task.isCancelled, isCurrent() {
            let held = reads.applied(to: world())
            if Self.waitsForRunner(open, in: held) { since = now() }
            switch DestinationResolver.resolve(
                open.destination, arrival: open.arrival, in: held,
                elapsed: now().timeIntervalSince(since), deadline: open.deadline,
                interrupted: interrupted())
            {
            case .open(let destination, _):
                land(destination)
                return .opened
            case .stay(let note):
                return .stayed(note)
            case .wait, .connect:
                // Every runner this Mac has is dialed already, so a runner that's
                // coming is waited for, and what it holds is asked of it once.
                let owed = reads.owed(for: open.destination, in: held)
                guard !owed.isEmpty else {
                    await pause()
                    continue
                }
                for (host, request) in owed {
                    reads.markAsked(host, request)
                    let data = await read(host, request)
                    guard isCurrent() else { return .cancelled }
                    reads.record(data, for: request, on: host)
                }
            }
        }
        return .cancelled
    }

    /// Whether a relaunch is still waiting for its window's runner (ov-279):
    /// the runner is configured here and hasn't connected. Its window keeps
    /// the place it had, showing that it's connecting, however long the
    /// runner takes, after sleep or over a slow tunnel; the ten seconds start
    /// once it's up. A runner gone from the configuration has no seat, and
    /// the restore falls back as before.
    nonisolated static func waitsForRunner(_ open: DestinationOpen, in world: DestinationResolver.World) -> Bool {
        guard open.arrival == .restore, let host = open.destination.runner.host else { return false }
        return world.seats.contains { $0.host == host && !$0.ready }
    }

    /// A window's half of a click: wait until this window may open it (`claim`),
    /// then `run` it.
    func drive(
        _ open: DestinationOpen, window: UUID, isKey: () -> Bool,
        world: () -> DestinationResolver.World,
        read: (_ host: String, _ request: DestinationReads.Request) async -> Data?,
        land: (Destination) -> Void,
        now: () -> Date = { Date() },
        pause: () async -> Void = { try? await Task.sleep(for: .milliseconds(250)) }
    ) async {
        while !Task.isCancelled, pending?.id == open.id {
            guard claim(open, window: window, isKey: isKey(), now: now()) else {
                if claimedBy != nil { return }
                await pause()
                continue
            }
            let outcome = await Self.run(
                open, isCurrent: { pending?.id == open.id }, world: world, read: read, land: land, now: now,
                pause: pause)
            if case .stayed(let note) = outcome {
                NSLog("Far Cooler: a notification's subject wasn't opened (%@).", note?.rawValue ?? "left alone")  // not UI copy: a log line
            }
            finish(open)
            return
        }
    }
}

extension MacDestination.Runner {
    /// What `client` says about itself to the resolver.
    @MainActor
    init(_ client: DaemonClient) {
        self.init(
            host: client.target, runnerId: client.daemonBuild?.runnerId,
            ready: client.state.isUsable && client.daemonBuild != nil, loaded: client.hasLoaded)
    }
}
