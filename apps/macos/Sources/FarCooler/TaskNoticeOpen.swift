import AgentKit
import Foundation
import UserNotifications

/// A click on a task notice: the task it's about, to be opened as the
/// navigator opens it (ov-106).
///
/// A notice names its task by key and its runner by `Host.runner_id`, never by
/// anything a selection is made of: a task's id and its workspace's id are
/// read from the runner (`task show`) once that runner is connected. A click
/// can come before that, on a cold launch from the notice or with the runner
/// still dialing, so the open waits for it, at most `waitsAtMost`, and then
/// leaves the app where the click brought it: in front.
struct TaskNoticeOpen: Equatable, Sendable {
    /// The task's key.
    var key: String
    /// The runner's `Host.runner_id`, as the notice says it.
    var runner: String?
    /// The runner as this app reaches it (`DaemonClient.target`), when the
    /// notice was posted here and says which: `""` is this Mac.
    var host: String?
    /// When the click came, for `waitsAtMost`.
    var since: Date
    /// Tells two clicks on the same notice apart, so the second is opened
    /// again rather than taken for the first.
    var id = UUID()

    /// How long a click waits for its runner and task. A runner this Mac
    /// reaches over ssh answers within seconds; a task that turned up half a
    /// minute later would move the window under somebody already doing
    /// something else.
    static let waitsAtMost: TimeInterval = 30

    /// The open a response asks for: a plain click on a task notice. Nil for
    /// an answer button, "Answer…" and a dismissal, and for a notice about no
    /// task. `target` is the local post's `DaemonClient.target`.
    init?(notice: TaskNotice, target: String?, action: String, now: Date) {
        guard action == UNNotificationDefaultActionIdentifier else { return nil }
        self.key = notice.key
        self.runner = notice.runner ?? notice.noticeId.flatMap(Self.parse(noticeId:))?.runner
        self.host = target
        self.since = now
    }

    /// `t:<runner id>:<task key>`, the notice id's long form, as its parts.
    /// Nil for anything else, including the hashed form (`t:<16 hex>`) the
    /// runner uses when the long one would pass 64 bytes, which names no
    /// runner and no key.
    static func parse(noticeId: String) -> (runner: String, key: String)? {
        let parts = noticeId.split(separator: ":", maxSplits: 2, omittingEmptySubsequences: false)
        guard parts.count == 3, parts[0] == "t", !parts[1].isEmpty, !parts[2].isEmpty else { return nil }
        return (String(parts[1]), String(parts[2]))
    }

    /// One runner this window holds, as `step` reads it.
    struct Seat: Equatable, Sendable {
        /// `DaemonClient.target`.
        var host: String
        /// Its `Host.runner_id`, once read.
        var runnerId: String?
        /// Connected, and this link's `status` read: what `runnerId` says is
        /// so, and `task show` will be answered.
        var ready: Bool
    }

    /// What to do next.
    enum Step: Equatable, Sendable {
        /// The runner isn't connected yet, or could still be one not yet read.
        case wait
        /// Read the task from this runner.
        case read(host: String)
        /// Leave the app in front, where the click put it.
        case giveUp
    }

    /// The runner to read the task from, given what this window holds now.
    ///
    /// The local post's runner first, by its target. Else the runner whose
    /// `runnerId` is the notice's, waiting while any runner hasn't said yet,
    /// since that one could be it; with every runner read and none of them
    /// it, there's nowhere to open it. Past `waitsAtMost` it gives up.
    func step(seats: [Seat], now: Date) -> Step {
        guard now.timeIntervalSince(since) < Self.waitsAtMost else { return .giveUp }
        if let host, let seat = seats.first(where: { $0.host == host }) {
            return seat.ready ? .read(host: host) : .wait
        }
        guard let runner = runner?.lowercased() else { return .giveUp }
        if let seat = seats.first(where: { $0.ready && $0.runnerId?.lowercased() == runner }) {
            return .read(host: seat.host)
        }
        return seats.contains(where: { !$0.ready }) ? .wait : .giveUp
    }

    /// Where `task show --json` says the task is: its id, and its board,
    /// which is its workspace or, on a runner without workspaces, its
    /// repository's implicit one, whose id is the repository's
    /// (`ContentView.Selection.workspace`).
    struct Place: Equatable, Sendable {
        var task: String
        var workspace: String

        init(task: String, workspace: String) {
            self.task = task
            self.workspace = workspace
        }

        /// Nil for a read that names no task, or one under another key.
        init?(show data: Data, key: String) {
            guard let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
                let task = body["task"] as? [String: Any],
                task["key"] as? String == key,
                let id = task["id"] as? String, !id.isEmpty
            else { return nil }
            let workspace = (task["workspace"] as? String).flatMap { $0.isEmpty ? nil : $0 }
            guard let board = workspace ?? (task["repository_id"] as? String), !board.isEmpty else { return nil }
            self.init(task: id, workspace: board)
        }
    }
}

/// The click waiting to be opened, shared by every window: the notification
/// center's delegate is the app's, and which window opens it is the windows'
/// to settle (`claim`).
@MainActor
final class TaskNoticeOpener: ObservableObject {
    static let shared = TaskNoticeOpener()

    /// The click not yet opened or given up on.
    @Published private(set) var pending: TaskNoticeOpen?
    /// The window that took `pending` on.
    private var claimedBy: UUID?

    /// How long a window that isn't key leaves a click to the key one.
    static let keyWindowFirst: TimeInterval = 0.5

    /// A click to open, in place of any before it.
    func request(_ open: TaskNoticeOpen) {
        pending = open
        claimedBy = nil
    }

    /// Whether `window` opens `open`: the first to ask, except that a window
    /// that isn't key waits `keyWindowFirst` for the key one.
    func claim(_ open: TaskNoticeOpen, window: UUID, isKey: Bool, now: Date) -> Bool {
        guard pending?.id == open.id else { return false }
        if let claimedBy { return claimedBy == window }
        guard isKey || now.timeIntervalSince(open.since) >= Self.keyWindowFirst else { return false }
        claimedBy = window
        return true
    }

    /// `open` is done with: opened, or given up on.
    func finish(_ open: TaskNoticeOpen) {
        guard pending?.id == open.id else { return }
        pending = nil
        claimedBy = nil
    }

    /// A window's half, with the runners it holds: `clients` is the
    /// window's `FleetStore.clients`, asked afresh on every pass.
    func drive(
        _ open: TaskNoticeOpen, window: UUID, isKey: () -> Bool, clients: () -> [String: DaemonClient],
        land: (_ host: String, _ place: TaskNoticeOpen.Place) -> Void,
        pause: () async -> Void = { try? await Task.sleep(for: .milliseconds(250)) }
    ) async {
        await drive(
            open, window: window, isKey: isKey, seats: { clients().values.map { TaskNoticeOpen.Seat($0) } },
            read: { host, key in await clients()[host]?.taskByKey(key).data }, land: land, pause: pause)
    }

    /// A window's half: wait until `open`'s runner is ready, read the task
    /// from it, and hand its place to `land`, which is the navigator's own
    /// way of opening a task. Gives up, leaving the window as it is, when
    /// the runner or the task can't be found in time, or the read fails.
    ///
    /// `seats` is read afresh on every pass, since runners come up on their
    /// own schedules; `pause` is the wait between passes.
    func drive(
        _ open: TaskNoticeOpen, window: UUID, isKey: () -> Bool, seats: () -> [TaskNoticeOpen.Seat],
        read: (_ host: String, _ key: String) async -> Data?,
        land: (_ host: String, _ place: TaskNoticeOpen.Place) -> Void,
        now: () -> Date = { Date() },
        pause: () async -> Void = { try? await Task.sleep(for: .milliseconds(250)) }
    ) async {
        while !Task.isCancelled, pending?.id == open.id {
            guard claim(open, window: window, isKey: isKey(), now: now()) else {
                if claimedBy != nil { return }
                await pause()
                continue
            }
            switch open.step(seats: seats(), now: now()) {
            case .wait:
                await pause()
            case .giveUp:
                NSLog("Far Cooler: a notice about %@ found no runner to open it on.", open.key)
                finish(open)
            case .read(let host):
                let data = await read(host, open.key)
                guard pending?.id == open.id else { return }
                // The deadline holds for the read too: a task that turns up
                // after it would move a window somebody has gone on using.
                if now().timeIntervalSince(open.since) >= TaskNoticeOpen.waitsAtMost {
                    NSLog("Far Cooler: a notice's task %@ was read too late to open.", open.key)
                } else if let data, let place = TaskNoticeOpen.Place(show: data, key: open.key) {
                    land(host, place)
                } else {
                    NSLog("Far Cooler: a notice's task %@ couldn't be read.", open.key)
                }
                finish(open)
            }
        }
    }
}

extension TaskNoticeOpen.Seat {
    /// What `client` says about itself to `step`.
    @MainActor
    init(_ client: DaemonClient) {
        self.init(
            host: client.target, runnerId: client.daemonBuild?.runnerId,
            ready: client.state.isUsable && client.daemonBuild != nil)
    }
}
