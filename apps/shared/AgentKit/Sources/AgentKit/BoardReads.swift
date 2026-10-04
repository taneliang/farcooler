import Foundation

/// What this person has read on one board (ov-104): the Unread section's
/// rule, and the Done rule's (ov-103).
///
/// Kept per ticket as a moment, not per item: opening a ticket reads every
/// item on it, its finish, its filing and all its notes, so the one thing
/// worth keeping is when it was last opened. An item is unread while it's
/// newer than that, and newer than the board's `floor`, before which
/// everything counts read (a first look shows the last day, not the whole
/// board's history; Mark All as Read moves it to now).
///
/// Per device for now, through `BoardReadStore`, so a runner-synced store
/// can stand in later without touching the rule.
public struct BoardReads: Equatable, Sendable {
    /// Everything at or before this counts read.
    public var floor: Date
    /// Each ticket's read mark, by task id: when it was last opened.
    public var opened: [String: Date]

    public init(floor: Date, opened: [String: Date] = [:]) {
        self.floor = floor
        self.opened = opened
    }

    /// Before anything was read: the last day counts unread.
    public static func firstLook(now: Date) -> BoardReads {
        BoardReads(floor: now.addingTimeInterval(-24 * 60 * 60))
    }

    /// Everything on `taskID` at or before this counts read.
    public func mark(for taskID: String) -> Date {
        max(floor, opened[taskID] ?? .distantPast)
    }

    /// Whether something that happened on `taskID` at `at` is unread.
    public func isUnread(_ taskID: String, at: Date) -> Bool { at > mark(for: taskID) }

    /// Whether `row` finished and nobody has opened it since.
    public func finishedUnread(_ row: TaskRow) -> Bool {
        row.status.isFinished && isUnread(row.id, at: row.statusSince)
    }

    /// `row` was opened: everything on it so far is read. The later of
    /// this device's clock and the runner's own last word on it (`lastMoved`,
    /// `latest`), so a runner whose clock runs ahead can't leave an item
    /// unread under a ticket just opened.
    public mutating func open(_ row: TaskRow, latest: Date? = nil, now: Date) {
        opened[row.id] = [now, row.lastMoved, latest ?? .distantPast].max()!
    }

    /// Mark All as Read: everything on `rows` so far, with the runner's
    /// clock allowed for as `open` does.
    public mutating func markAllRead(rows: [TaskRow], now: Date) {
        floor = ([now] + rows.map(\.lastMoved)).max()!
        opened = [:]
    }

    /// Without the marks the floor has passed, which say nothing more.
    public func pruned() -> BoardReads {
        BoardReads(floor: floor, opened: opened.filter { $0.value > floor })
    }
}

/// Where a board's read state is kept. This device's defaults for now
/// (`DefaultsBoardReads`); a runner-synced store later.
public protocol BoardReadStore {
    func load(host: String, workspace: String, now: Date) -> BoardReads
    func save(_ reads: BoardReads, host: String, workspace: String)
    /// Whether this device's state was sent up to a runner that keeps it
    /// (ov-113), once, so it isn't again.
    func isUploaded(host: String, workspace: String) -> Bool
    func markUploaded(host: String, workspace: String)
    /// What this device kept for the board, as opposed to what `load` made
    /// up: marks, and a floor only if someone set it (Mark All as Read, or
    /// state from before the floor's origin was noted). A first look nobody
    /// saw as Unread is no floor here. Nil when nothing real is kept.
    func keptReads(host: String, workspace: String) -> BoardReads?
    /// Marks made and not yet acknowledged by the runner (ov-113), kept so
    /// they survive a relaunch.
    func loadPending(host: String, workspace: String) -> ReadsRaise
    func savePending(_ pending: ReadsRaise, host: String, workspace: String)
    /// Whether the floor was set on purpose on this device (a Mark All as Read
    /// that was kept), as a phone counts it; false for one an older build or a
    /// first look left. The Mac doesn't ask, and a store that can't say answers yes.
    func floorWasSet(host: String, workspace: String) -> Bool
    func markFloorSet(host: String, workspace: String)
}

extension BoardReadStore {
    public func floorWasSet(host: String, workspace: String) -> Bool { true }
    public func markFloorSet(host: String, workspace: String) {}
}

/// The read state in this device's defaults, under
/// `board.read.<runner>.<workspace>`: a floor, and each opened ticket's
/// mark keyed by its task id.
public struct DefaultsBoardReads: BoardReadStore {
    public let defaults: UserDefaults

    public init(_ defaults: UserDefaults = .standard) { self.defaults = defaults }

    public static func floorKey(host: String, workspace: String) -> String {
        "board.read.\(host).\(workspace).floor"
    }

    public static func openedKey(host: String, workspace: String) -> String {
        "board.read.\(host).\(workspace).opened"
    }

    /// What's kept, or, the first time, the board's last visit (what Since
    /// Last Visit kept) if it's within the last day, else the last day,
    /// written so it holds still.
    public func load(host: String, workspace: String, now: Date) -> BoardReads {
        let floorKey = Self.floorKey(host: host, workspace: workspace)
        guard defaults.object(forKey: floorKey) != nil else {
            // Never further back than a first look: a board last visited
            // weeks ago would otherwise call every task finished since
            // unread, and Done would list them all (ov-104 review).
            let visit = BoardVisit.read(host: host, workspace: workspace, from: defaults)
            let look = BoardReads.firstLook(now: now).floor
            let first = BoardReads(floor: max(visit ?? .distantPast, look))
            save(first, host: host, workspace: workspace)
            // Made up here, not read by anyone, unless it is a real visit.
            defaults.set(first.floor == look, forKey: Self.inventedKey(host: host, workspace: workspace))
            return first
        }
        let floor = Date(timeIntervalSince1970: defaults.double(forKey: floorKey))
        let raw = defaults.dictionary(forKey: Self.openedKey(host: host, workspace: workspace)) ?? [:]
        let opened = raw.compactMapValues { ($0 as? Double).map(Date.init(timeIntervalSince1970:)) }
        return BoardReads(floor: floor, opened: opened)
    }

    public static func uploadedKey(host: String, workspace: String) -> String {
        "board.read.\(host).\(workspace).synced"
    }

    public func isUploaded(host: String, workspace: String) -> Bool {
        defaults.bool(forKey: Self.uploadedKey(host: host, workspace: workspace))
    }

    static func inventedKey(host: String, workspace: String) -> String {
        "board.read.\(host).\(workspace).floor.invented"
    }

    public func keptReads(host: String, workspace: String) -> BoardReads? {
        guard defaults.object(forKey: Self.floorKey(host: host, workspace: workspace)) != nil else { return nil }
        let state = load(host: host, workspace: workspace, now: Date())
        guard defaults.bool(forKey: Self.inventedKey(host: host, workspace: workspace)) else { return state }
        let marks = BoardReads(floor: .distantPast, opened: state.opened)
        return marks.opened.isEmpty ? nil : marks
    }

    public func loadPending(host: String, workspace: String) -> ReadsRaise {
        let key = "board.read.\(host).\(workspace).pending"
        let floor = defaults.object(forKey: key + ".floor") as? Double
        let raw = defaults.dictionary(forKey: key + ".opened") ?? [:]
        return ReadsRaise(
            floor: floor.map(Date.init(timeIntervalSince1970:)),
            opened: raw.compactMapValues { ($0 as? Double).map(Date.init(timeIntervalSince1970:)) })
    }

    public func savePending(_ pending: ReadsRaise, host: String, workspace: String) {
        let key = "board.read.\(host).\(workspace).pending"
        defaults.set(pending.floor?.timeIntervalSince1970, forKey: key + ".floor")
        defaults.set(pending.opened.mapValues(\.timeIntervalSince1970), forKey: key + ".opened")
    }

    public func floorWasSet(host: String, workspace: String) -> Bool {
        defaults.bool(forKey: "board.read.\(host).\(workspace).floor.set")
    }

    public func markFloorSet(host: String, workspace: String) {
        defaults.set(true, forKey: "board.read.\(host).\(workspace).floor.set")
    }

    public func markUploaded(host: String, workspace: String) {
        defaults.set(true, forKey: Self.uploadedKey(host: host, workspace: workspace))
    }

    public func save(_ reads: BoardReads, host: String, workspace: String) {
        let kept = reads.pruned()
        // A floor that moved is somebody's doing, no longer made up.
        let floorKey = Self.floorKey(host: host, workspace: workspace)
        if defaults.object(forKey: floorKey) as? Double != kept.floor.timeIntervalSince1970 {
            defaults.removeObject(forKey: Self.inventedKey(host: host, workspace: workspace))
        }
        defaults.set(kept.floor.timeIntervalSince1970, forKey: Self.floorKey(host: host, workspace: workspace))
        defaults.set(
            kept.opened.mapValues(\.timeIntervalSince1970), forKey: Self.openedKey(host: host, workspace: workspace))
    }
}
