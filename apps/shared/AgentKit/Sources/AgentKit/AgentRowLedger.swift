import Foundation
import Synchronization

/// What one page or follow changed, ready for `AgentRowStore.apply` to put on
/// screen in time proportional to the change rather than to the list.
///
/// Built off the main thread by `AgentRowLedger`, which holds the whole
/// window. The main thread only sets the rows named here into their boxes and,
/// when the set of rows changed, swaps in an order array the ledger already
/// built: it never decodes, merges, sorts or walks the list (ov-371).
public struct AgentRowDelta: Sendable, Equatable {
    /// Every held row's id, oldest first, when a row arrived, went or moved;
    /// nil when only the contents of held rows changed.
    public var order: [String]?
    /// `order` as a transcript draws it, runs of tool calls folded
    /// (`AgentConversation.items`, ov-452), with `order` and only then.
    public var items: [ConversationItem]?
    /// Rows that arrived or whose contents changed.
    public var rows: [AgentRow] = []
    /// Rows no longer held.
    public var removed: [String] = []
    public var epoch: UInt64 = 0
    public var rev: UInt64 = 0
    public var moreBefore: Bool = false
    /// The ledger's version before this change and after it (ov-612). A
    /// view that has applied every change up to `base` can apply this one;
    /// one that hasn't has missed something and takes the whole window.
    public var base: UInt64 = 0
    public var version: UInt64 = 0
    /// The ledger's whole window rather than a change (ov-612): `rows` is
    /// every held row in `order`, and a row the view holds that isn't among
    /// them is gone. What a start and a re-page hand the view, so whatever
    /// it missed, it matches the ledger again.
    public var complete: Bool = false
    /// Rows were put above the oldest held (an older page): the reader's
    /// place needs holding while they come in (ov-629).
    public var prepends: Bool = false

    public var isEmpty: Bool { order == nil && rows.isEmpty && removed.isEmpty }
}

/// The rows a returning pane draws first, and the cursor to follow from.
public struct AgentRowSnapshot: Sendable, Equatable, Codable {
    public var epoch: UInt64
    public var rev: UInt64
    public var moreBefore: Bool
    /// Oldest first.
    public var rows: [AgentRow]

    public init(epoch: UInt64, rev: UInt64, moreBefore: Bool, rows: [AgentRow]) {
        self.epoch = epoch
        self.rev = rev
        self.moreBefore = moreBefore
        self.rows = rows
    }
}

/// A terminal's rows as the client holds them, off the main thread (ov-371).
///
/// Decodes every page and follow, applies follow diffs by id, and hands the
/// main thread an `AgentRowDelta` naming only what changed. An actor, so the
/// work happens on the cooperative pool, never the main actor.
///
/// Each follow loop writes under a generation (ov-612). A start or a stop
/// moves the generation on at once, from the main actor, and a write from an
/// older one is refused before it changes anything: the client core's call
/// can't be cancelled, so a stopped loop's follow can still answer seconds
/// later, and its rows must not land in a window the view will never hear
/// about.
public actor AgentRowLedger {
    private var rows: [String: AgentRow] = [:]
    private var order: [String] = []
    private var epoch: UInt64 = 0
    private var rev: UInt64 = 0
    private var moreBefore = false
    /// Moves on with every change a view would draw (ov-612): the rows, their
    /// order, `moreBefore`. Not the cursor alone.
    public private(set) var version: UInt64 = 0
    private let generation = Mutex<UInt64>(0)
    /// Writes refused as stale, for tests to wait on.
    private(set) var refused = 0

    public init() {}

    /// A write from a loop that a newer start or a stop has replaced.
    public struct Stale: Error, Equatable {}

    /// Begin a new generation, so every write under an older one is refused
    /// from now on. Called by `AgentRowStore.start` and `stop`.
    @discardableResult
    public nonisolated func advance() -> UInt64 {
        generation.withLock { current in
            current += 1
            return current
        }
    }

    /// Whether `generation` is still the newest.
    public nonisolated func isCurrent(_ generation: UInt64) -> Bool {
        self.generation.withLock { $0 == generation }
    }

    /// Refuse a write under a replaced generation; nil writes always (a test,
    /// or a caller with no loop).
    private func admit(_ generation: UInt64?) throws {
        guard let generation, !isCurrent(generation) else { return }
        refused += 1
        throw Stale()
    }

    /// The cursor a follow continues from, or nil before anything was held.
    public var cursor: (epoch: UInt64, rev: UInt64)? {
        epoch == 0 && order.isEmpty ? nil : (epoch, rev)
    }

    /// The oldest held row's `ord`, which an older page is asked `before`.
    public var oldestOrd: UInt64? { order.first.flatMap { rows[$0]?.ord } }

    public var count: Int { order.count }

    /// What a follow answered, decoded and applied.
    public enum Followed: Sendable, Equatable {
        case delta(AgentRowDelta)
        /// The runner can't say what changed since our cursor: page again.
        case reset
    }

    // MARK: Pages

    /// `agent.rows`'s answer, the newest rows: replaces what is held unless it
    /// is the same projection, in which case rows older than the page stay.
    public func page(_ data: Data, generation: UInt64? = nil) throws -> AgentRowDelta {
        let page = try AgentRowPage.decode(data)
        try admit(generation)
        return replace(with: page)
    }

    public func replace(with page: AgentRowPage) -> AgentRowDelta {
        let base = version
        let samePlace = page.epoch == epoch && !order.isEmpty
        let floor = page.rows.first?.ord ?? UInt64.max
        // Rows below the page are kept when the projection is the same one:
        // `ord` never moves, so they are still where they were.
        let kept = samePlace ? order.filter { (rows[$0]?.ord ?? 0) < floor } : []
        var next: [String: AgentRow] = [:]
        next.reserveCapacity(kept.count + page.rows.count)
        for id in kept { next[id] = rows[id] }
        var delta = AgentRowDelta()
        for row in page.rows {
            next[row.id] = row
            if rows[row.id] != row { delta.rows.append(row) }
        }
        let nextOrder = kept + page.rows.map(\.id)
        delta.removed = order.filter { next[$0] == nil }
        rows = next
        order = nextOrder
        epoch = page.epoch
        rev = page.rev
        moreBefore = samePlace && !kept.isEmpty ? moreBefore : page.moreBefore
        delta.order = nextOrder
        version += 1
        return stamped(delta, from: base)
    }

    /// An older page (`agent.rows {before: oldestOrd}`), put above what is
    /// held. A page from another projection is dropped: the follow will
    /// reset.
    public func older(_ data: Data) throws -> AgentRowDelta {
        let page = try AgentRowPage.decode(data)
        guard page.epoch == epoch else { return stamped(AgentRowDelta()) }
        let floor = oldestOrd ?? UInt64.max
        let fresh = page.rows.filter { $0.ord < floor && rows[$0.id] == nil }
        var delta = AgentRowDelta()
        let base = version
        if fresh.isEmpty && moreBefore == page.moreBefore { return stamped(delta) }
        moreBefore = page.moreBefore
        version += 1
        guard !fresh.isEmpty else { return stamped(delta, from: base) }
        for row in fresh { rows[row.id] = row }
        order = fresh.map(\.id) + order
        delta.rows = fresh
        delta.order = order
        delta.prepends = true
        return stamped(delta, from: base)
    }

    // MARK: Follows

    /// `agent.rows_follow`'s answer, applied by id.
    public func follow(_ data: Data, generation: UInt64? = nil) throws -> Followed {
        let changes = try AgentRowChanges.decode(data)
        try admit(generation)
        return apply(changes)
    }

    public func apply(_ changes: AgentRowChanges) -> Followed {
        if changes.reset || changes.epoch != epoch { return .reset }
        var delta = AgentRowDelta()
        var appended: [AgentRow] = []
        var reorder = false
        let newest = order.last.flatMap { rows[$0]?.ord }
        for change in changes.changes {
            switch change {
            case .insert(let row), .update(let row):
                if let held = rows[row.id] {
                    guard held != row else { continue }
                    rows[row.id] = row
                    delta.rows.append(row)
                } else if newest.map({ row.ord > $0 }) ?? true {
                    // New below everything held. A row older than the window
                    // is someone else's page and isn't drawn yet.
                    rows[row.id] = row
                    appended.append(row)
                    delta.rows.append(row)
                }
            case .remove(let id, _):
                guard rows.removeValue(forKey: id) != nil else { continue }
                delta.removed.append(id)
                reorder = true
            }
        }
        if !appended.isEmpty || reorder {
            if reorder {
                let gone = Set(delta.removed)
                order.removeAll { gone.contains($0) }
            }
            appended.sort { $0.ord < $1.ord }
            order.append(contentsOf: appended.map(\.id))
            delta.order = order
        }
        rev = changes.rev
        // The cursor alone moving is nothing a view draws.
        guard !delta.isEmpty else { return .delta(stamped(delta)) }
        let base = version
        version += 1
        return .delta(stamped(delta, from: base))
    }

    // MARK: Cache

    /// Take up what a cache held, so a follow continues from its cursor.
    public func restore(_ snapshot: AgentRowSnapshot, generation: UInt64? = nil) throws -> AgentRowDelta {
        try admit(generation)
        return replace(with: AgentRowPage(epoch: snapshot.epoch, rev: snapshot.rev, moreBefore: snapshot.moreBefore, rows: snapshot.rows))
    }

    /// Everything held, as a complete delta (ov-612): what makes a view
    /// match the ledger again whatever it missed.
    public func window() -> AgentRowDelta {
        var delta = AgentRowDelta(order: order, rows: order.compactMap { rows[$0] })
        delta.complete = true
        return stamped(delta)
    }

    /// The newest `limit` rows and the cursor, for the cache.
    public func snapshot(newest limit: Int) -> AgentRowSnapshot {
        let ids = order.suffix(limit)
        return AgentRowSnapshot(
            epoch: epoch, rev: rev, moreBefore: moreBefore || ids.count < order.count,
            rows: ids.compactMap { rows[$0] })
    }

    /// `delta` with what it says about the window, made from version `base`
    /// (no change: the version it's at).
    private func stamped(_ delta: AgentRowDelta, from base: UInt64? = nil) -> AgentRowDelta {
        var delta = delta
        delta.base = base ?? version
        delta.version = version
        if let order = delta.order {
            delta.items = Self.items(order) { rows[$0] }
        }
        delta.epoch = epoch
        delta.rev = rev
        delta.moreBefore = moreBefore
        return delta
    }

    /// `order`'s items, the hint left out (it's the composer's): what a
    /// row's kind says about its grouping never changes, so they're rebuilt
    /// only when the order does, here, off the main thread.
    static func items(_ order: [String], row: (String) -> AgentRow?) -> [ConversationItem] {
        AgentConversation.items(order.filter { $0 != AgentRow.hintID }) { id in
            row(id).map { AgentConversation.groupRole($0.kind) } ?? .other
        }
    }
}
