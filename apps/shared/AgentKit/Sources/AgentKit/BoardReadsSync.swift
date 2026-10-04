import Foundation

// Read state kept on the runner (ov-113): the wire shape a runner with
// `board_reads` sends, and the rules for merging it with what a device has.
//
// Every value only rises, so merging is order-free and a repeat is harmless.
// Times are the runner's clock; a device never adds its own `now` to a shared
// mark, which a clock running ahead would turn into news hidden everywhere.

/// One board's read state as a runner sends it: `task list --json`'s `reads`,
/// `board mark-read`'s answer and the `reads` event's line.
public struct WireBoardReads: Decodable, Equatable, Sendable {
    public struct Mark: Decodable, Equatable, Sendable {
        public var taskID: String
        public var openedMs: Int64

        public init(taskID: String, openedMs: Int64) {
            self.taskID = taskID
            self.openedMs = openedMs
        }

        enum CodingKeys: String, CodingKey {
            case taskID = "task_id"
            case openedMs = "opened_ms"
        }
    }

    public var workspaceID: String
    public var floorMs: Int64
    public var opened: [Mark]

    public init(workspaceID: String, floorMs: Int64, opened: [Mark] = []) {
        self.workspaceID = workspaceID
        self.floorMs = floorMs
        self.opened = opened
    }

    enum CodingKeys: String, CodingKey {
        case workspaceID = "workspace_id"
        case floorMs = "floor_ms"
        case opened
    }

    /// The state as the rule here keeps it.
    public var reads: BoardReads {
        BoardReads(
            floor: Self.date(floorMs),
            opened: Dictionary(opened.map { ($0.taskID, Self.date($0.openedMs)) }, uniquingKeysWith: max)
        ).pruned()
    }

    static func date(_ ms: Int64) -> Date { Date(timeIntervalSince1970: Double(ms) / 1000) }

    /// The `reads` of a `task list --json` board, or nil from a runner that
    /// sends none (or a board that named no workspace). A malformed one is
    /// nil too: it costs the read state, never the board.
    public static func decode(board data: Data) -> WireBoardReads? {
        struct Board: Decodable { var reads: WireBoardReads? }
        return (try? JSONDecoder().decode(Board.self, from: data))?.reads
    }

    /// A bare state, as `board mark-read --json` prints it.
    public static func decode(state data: Data) -> WireBoardReads? {
        try? JSONDecoder().decode(WireBoardReads.self, from: data)
    }
}

extension BoardReads {
    /// Both devices' words together: each mark and the floor, the later of
    /// the two, then without what the floor has passed.
    public func merged(with other: BoardReads) -> BoardReads {
        BoardReads(
            floor: max(floor, other.floor),
            opened: opened.merging(other.opened, uniquingKeysWith: max)
        ).pruned()
    }

    /// `row` was opened, as far as the runner has told us: through its last
    /// word on it (`lastMoved`) and the newest note read (`latest`). No device
    /// clock, so a Mac running ahead can't hide what the runner writes next.
    public mutating func open(_ row: TaskRow, seenThrough latest: Date?) {
        opened[row.id] = max(opened[row.id] ?? .distantPast, row.lastMoved, latest ?? .distantPast)
    }

    /// Mark All as Read through what was shown: the rows' last words and the
    /// notes read. Raises the floor, never lowers it, and keeps no marks the
    /// floor has passed.
    public mutating func markAllRead(rows: [TaskRow], seenThrough latest: Date?) {
        floor = max(floor, rows.map(\.lastMoved).max() ?? .distantPast, latest ?? .distantPast)
        opened = opened.filter { $0.value > floor }
    }

    /// Milliseconds since 1970, the runner's unit.
    public static func milliseconds(_ date: Date) -> Int64 { Int64((date.timeIntervalSince1970 * 1000).rounded()) }
}

/// What to tell a runner: `board mark-read`'s `--floor` and `--task`s.
public struct ReadsRaise: Equatable, Sendable {
    /// Mark All as Read through this, when it is one.
    public var floor: Date?
    public var opened: [String: Date]

    public init(floor: Date? = nil, opened: [String: Date] = [:]) {
        self.floor = floor
        self.opened = opened
    }

    public var isEmpty: Bool { floor == nil && opened.isEmpty }

    /// Both, each value the later.
    public func merging(_ other: ReadsRaise) -> ReadsRaise {
        ReadsRaise(
            floor: [floor, other.floor].compactMap { $0 }.max(),
            opened: opened.merging(other.opened, uniquingKeysWith: max))
    }

    /// `reads` with this raised into it.
    public func applied(to reads: BoardReads) -> BoardReads {
        reads.merged(with: BoardReads(floor: floor ?? .distantPast, opened: opened))
    }

    /// What is left of this once `sent` has gone: nothing the runner was
    /// told at or above its value.
    public func without(_ sent: ReadsRaise) -> ReadsRaise {
        var left = floor
        if let f = floor, let told = sent.floor, f <= told { left = nil }
        return ReadsRaise(
            floor: left,
            opened: opened.filter { $0.value > (sent.opened[$0.key] ?? .distantPast) })
    }

    /// The arguments after `board mark-read --repo R --workspace W`.
    public var arguments: [String] {
        var args: [String] = []
        for (id, at) in opened.sorted(by: { $0.key < $1.key }) {
            args += ["--task", "\(id):\(BoardReads.milliseconds(at))"]
        }
        if let floor { args += ["--floor", String(BoardReads.milliseconds(floor))] }
        return args
    }
}
