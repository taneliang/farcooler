import Foundation

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
    /// Rows that arrived or whose contents changed.
    public var rows: [AgentRow] = []
    /// Rows no longer held.
    public var removed: [String] = []
    public var epoch: UInt64 = 0
    public var rev: UInt64 = 0
    public var moreBefore: Bool = false

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
public actor AgentRowLedger {
    private var rows: [String: AgentRow] = [:]
    private var order: [String] = []
    private var epoch: UInt64 = 0
    private var rev: UInt64 = 0
    private var moreBefore = false

    public init() {}

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
    public func page(_ data: Data) throws -> AgentRowDelta {
        replace(with: try AgentRowPage.decode(data))
    }

    public func replace(with page: AgentRowPage) -> AgentRowDelta {
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
        return stamped(delta)
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
        moreBefore = page.moreBefore
        guard !fresh.isEmpty else { return stamped(delta) }
        for row in fresh { rows[row.id] = row }
        order = fresh.map(\.id) + order
        delta.rows = fresh
        delta.order = order
        return stamped(delta)
    }

    // MARK: Follows

    /// `agent.rows_follow`'s answer, applied by id.
    public func follow(_ data: Data) throws -> Followed {
        apply(try AgentRowChanges.decode(data))
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
        return .delta(stamped(delta))
    }

    // MARK: Cache

    /// Take up what a cache held, so a follow continues from its cursor.
    public func restore(_ snapshot: AgentRowSnapshot) -> AgentRowDelta {
        let delta = replace(with: AgentRowPage(epoch: snapshot.epoch, rev: snapshot.rev, moreBefore: snapshot.moreBefore, rows: snapshot.rows))
        return delta
    }

    /// The newest `limit` rows and the cursor, for the cache.
    public func snapshot(newest limit: Int) -> AgentRowSnapshot {
        let ids = order.suffix(limit)
        return AgentRowSnapshot(
            epoch: epoch, rev: rev, moreBefore: moreBefore || ids.count < order.count,
            rows: ids.compactMap { rows[$0] })
    }

    private func stamped(_ delta: AgentRowDelta) -> AgentRowDelta {
        var delta = delta
        delta.epoch = epoch
        delta.rev = rev
        delta.moreBefore = moreBefore
        return delta
    }
}
