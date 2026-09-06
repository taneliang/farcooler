import Foundation

/// Where a dragged worktree card lands, worked out without a view.
///
/// Shared because both Apple apps drag the same cards into the same order and
/// send it to the same runner. Two copies of this arithmetic would be two apps
/// that disagree about where a card went — visibly, on the same fleet, the
/// moment somebody drags on one and looks at the other.
///
/// Pure on purpose. Drag-and-drop is the part of a UI that is hardest to look
/// at and be sure about: the pointer is moving, the list is re-laying out, and
/// the difference between "dropped above" and "dropped below" is a few pixels
/// nobody can see in a screenshot. So the decision lives here, where it can be
/// asked the same question a hundred times, and the views are left holding only
/// the gesture.
///
/// It knows nothing about activity, attention or recency, and it must never
/// learn: the whole point of storing an order is that a card stays where it was
/// put. See `crates/store/src/migrate.rs`'s migration 0009 for the durable half.
public enum WorkspaceOrder {

    /// Which side of the row under the pointer a drop would go.
    public enum Edge: Sendable, Equatable {
        case above
        case below
    }

    /// The half of a row the pointer is in.
    ///
    /// A midpoint rather than a margin at each end. A margin leaves a dead band
    /// through the middle of every row where a drop means nothing, and what a
    /// person does with a card is aim at the gap between two others — so the
    /// gesture would fail most often exactly where it is aimed most carefully.
    ///
    /// A row with no height reads as `.above`, which is the answer that cannot
    /// move a card past somewhere it has not been dragged.
    public static func edge(pointerY: Double, rowHeight: Double) -> Edge {
        guard rowHeight > 0 else { return .above }
        return pointerY < rowHeight / 2 ? .above : .below
    }

    /// `ids` with `dragged` lifted out and put back against `target`.
    ///
    /// Returns the list UNCHANGED when the drop would not move anything: the
    /// card dropped on itself, either id is not in the list, or the card is
    /// already exactly where the drop would put it. Callers lean on that —
    /// comparing the answer to what they started with is how they decide
    /// whether to spend a round trip, and a reorder request that changes
    /// nothing still makes every other client re-read the fleet.
    ///
    /// The target index is taken AFTER the dragged card is removed, which is
    /// what makes dragging downwards land where the pointer is rather than one
    /// short of it — the classic off-by-one in every hand-written reorder.
    public static func moved(
        _ ids: [String], dragging dragged: String, to target: String, _ edge: Edge
    ) -> [String] {
        guard dragged != target,
            let from = ids.firstIndex(of: dragged),
            ids.contains(target)
        else { return ids }

        var next = ids
        next.remove(at: from)
        guard let landing = next.firstIndex(of: target) else { return ids }
        next.insert(dragged, at: edge == .above ? landing : landing + 1)
        return next == ids ? ids : next
    }
}
