import Foundation

// The one tree from the keyboard (ov-321 review H1), as values: where ↑ and ↓
// go, what → and ← do, and what Return does, over the rows as drawn. Every
// row is reachable, groups with nowhere to go (No Theme, a done fold, Loose
// Worktrees) included; the views only send the keys here.

public enum OneTreeKeys {
    /// What a key does to the tree.
    public enum Move: Equatable, Sendable {
        /// The keyboard's cursor moves to this row.
        case cursor(String)
        /// This row opens or closes.
        case toggle(String)
        /// The window goes where this row points.
        case choose(String)
        /// Nothing to do: the key is the window's.
        case none
    }

    /// ↑ (`by` −1) or ↓ (1): the row above or below the cursor, held at
    /// either end; from no cursor, the first going down, the last going up.
    public static func step(_ rows: [OneTreeRow], from cursor: String?, by: Int) -> Move {
        guard !rows.isEmpty else { return .none }
        guard let at = cursor.flatMap({ id in rows.firstIndex { $0.id == id } }) else {
            return .cursor(by >= 0 ? rows[0].id : rows[rows.count - 1].id)
        }
        let next = min(max(at + by, 0), rows.count - 1)
        return next == at ? .none : .cursor(rows[next].id)
    }

    /// Whether arriving on a row by ↑ or ↓ goes where it points, as a
    /// list's selection does. Not a row that takes the keyboard away (the
    /// orchestrator, a subagent in it), nor one with nowhere to go.
    public static func choosesOnArrival(_ node: OneTreeNode) -> Bool {
        guard let target = node.target else { return false }
        return target != .orchestrator
    }

    /// →: a closed row opens; an open one moves to its first child.
    public static func right(_ rows: [OneTreeRow], cursor: String?) -> Move {
        guard let at = index(rows, cursor), rows[at].node.hasChildren else { return .none }
        if !rows[at].expanded { return .toggle(rows[at].id) }
        return at + 1 < rows.count ? .cursor(rows[at + 1].id) : .none
    }

    /// ←: an open row closes; otherwise the cursor goes to its parent.
    public static func left(_ rows: [OneTreeRow], cursor: String?) -> Move {
        guard let at = index(rows, cursor) else { return .none }
        if rows[at].expanded { return .toggle(rows[at].id) }
        let depth = rows[at].depth
        guard depth > 0, let parent = rows[..<at].lastIndex(where: { $0.depth < depth }) else { return .none }
        return .cursor(rows[parent].id)
    }

    /// Return: a row that goes somewhere is chosen (or, already chosen, is
    /// entered, which the window does); a group opens or closes.
    public static func enter(_ rows: [OneTreeRow], cursor: String?) -> Move {
        guard let at = index(rows, cursor) else { return .none }
        let row = rows[at]
        if row.node.target != nil { return .choose(row.id) }
        return row.node.hasChildren ? .toggle(row.id) : .none
    }

    private static func index(_ rows: [OneTreeRow], _ cursor: String?) -> Int? {
        cursor.flatMap { id in rows.firstIndex { $0.id == id } }
    }
}

/// The workspace's Needs You count: the one number the title bar and the
/// sidebar's Needs You row both say (ov-321 review H3).
public enum WorkspaceNeedsYou {
    /// `items` is the workspace's own Needs You list as the app holds it:
    /// the runner's, once read; derived from the fleet on a runner too old
    /// to serve one. `columnCount` is the board's Needs Decision column.
    /// `themeAsks` is the plan's themes with an ask for the owner.
    ///
    /// The runner's list, once read, is the whole answer. Until then, and
    /// always on a runner that serves none, the list can't hold the
    /// decisions, so the column counts them beside what was derived.
    public static func count(items: Int, columnCount: Int, listRead: Bool, listServed: Bool, themeAsks: Int) -> Int {
        (listRead && listServed ? items : columnCount + items) + themeAsks
    }
}
