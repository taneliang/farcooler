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
    /// Last Visit kept) else the last day, written so it holds still.
    public func load(host: String, workspace: String, now: Date) -> BoardReads {
        let floorKey = Self.floorKey(host: host, workspace: workspace)
        guard defaults.object(forKey: floorKey) != nil else {
            let first = BoardVisit.read(host: host, workspace: workspace, from: defaults)
                .map { BoardReads(floor: $0) } ?? .firstLook(now: now)
            save(first, host: host, workspace: workspace)
            return first
        }
        let floor = Date(timeIntervalSince1970: defaults.double(forKey: floorKey))
        let raw = defaults.dictionary(forKey: Self.openedKey(host: host, workspace: workspace)) ?? [:]
        let opened = raw.compactMapValues { ($0 as? Double).map(Date.init(timeIntervalSince1970:)) }
        return BoardReads(floor: floor, opened: opened)
    }

    public func save(_ reads: BoardReads, host: String, workspace: String) {
        let kept = reads.pruned()
        defaults.set(kept.floor.timeIntervalSince1970, forKey: Self.floorKey(host: host, workspace: workspace))
        defaults.set(
            kept.opened.mapValues(\.timeIntervalSince1970), forKey: Self.openedKey(host: host, workspace: workspace))
    }
}
