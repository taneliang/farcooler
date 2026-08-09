import Foundation

/// Which way a split divides its children.
enum Axis: String, Codable, Equatable {
    /// Children side by side.
    case horizontal
    /// Children stacked.
    case vertical
}

/// What a tile shows.
///
/// `tmux` is ONE tile, and tmux keeps its own tiling inside it. That is the
/// constraint that makes this layout affordable: `PaneGroup` stays a projection
/// of tmux's window tree, `layout.split` still owns where panes go, and nothing
/// here needs a protocol change.
///
/// A client-owned tree of individual terminals was the obvious alternative and
/// is the wrong one — it is the second pane tree migration 0003 deleted, and its
/// reasoning has not changed: tmux has had split trees, named layouts, dividers
/// and zoom for twenty years, it is already the authority for what is running,
/// and a second copy becomes a third in every client that draws it.
enum TileKind: String, Codable, Equatable, CaseIterable {
    case tmux
    case diff
    case pr

    /// Title case, because these name controls.
    var label: String {
        switch self {
        case .tmux: return "Agents"
        case .diff: return "Changes"
        case .pr: return "Pull Request"
        }
    }
}

/// A worktree's layout: a tree whose leaves are tiles.
indirect enum TileNode: Codable, Equatable {
    case leaf(TileKind)
    /// Children, and the fractions of the axis each one takes. The two arrays
    /// always have the same count.
    case split(Axis, [TileNode], [Double])

    /// Every tile kind present, in reading order.
    var kinds: [TileKind] {
        switch self {
        case .leaf(let k): return [k]
        case .split(_, let children, _): return children.flatMap(\.kinds)
        }
    }

    func contains(_ kind: TileKind) -> Bool { kinds.contains(kind) }

    /// Add a tile of this kind beside what is already there, or take it away.
    ///
    /// Toggling rather than free-form arrangement, deliberately. The layouts
    /// that earn their keep here are "agent and diff", "agent, diff and PR", and
    /// one of them alone; a general splitter would be more powerful and would
    /// mostly be used to rebuild the default by hand.
    func toggling(_ kind: TileKind) -> TileNode {
        if contains(kind) {
            // Removing the last tile would leave nothing to draw, so the other
            // half of the default takes over rather than the view going blank.
            return removing(kind) ?? .leaf(kind == .tmux ? .diff : .tmux)
        }
        return .split(.horizontal, [self, .leaf(kind)], [0.6, 0.4])
    }

    /// This tree without that kind, or nil when nothing would be left.
    private func removing(_ kind: TileKind) -> TileNode? {
        switch self {
        case .leaf(let k):
            return k == kind ? nil : self
        case .split(let axis, let children, _):
            let kept = children.compactMap { $0.removing(kind) }
            if kept.isEmpty { return nil }
            // A split with one child is not a split.
            if kept.count == 1 { return kept[0] }
            let share = 1.0 / Double(kept.count)
            return .split(axis, kept, Array(repeating: share, count: kept.count))
        }
    }
}

/// Turning a split's fractions into sizes, and a dragged divider back into
/// fractions.
///
/// Here rather than in the view because it is arithmetic, and arithmetic that
/// was wrong once: an `HSplitView` ignored the fractions entirely and sized
/// children by what they asked for, which handed the tmux canvas nearly the
/// whole row and left the diff tile a sliver.
enum TileSizing {
    /// Each child's size along the axis.
    ///
    /// Normalized rather than trusted to sum to one, so a layout written by a
    /// build whose arithmetic differed still fills the space exactly instead of
    /// leaving a gap or overflowing.
    static func sizes(_ fractions: [Double], usable: Double) -> [Double] {
        let sum = fractions.reduce(0, +)
        guard sum > 0, fractions.allSatisfy({ $0.isFinite }) else {
            return Array(repeating: usable / Double(fractions.count), count: fractions.count)
        }
        return fractions.map { usable * ($0 / sum) }
    }

    /// The fractions after dragging the divider that follows `index`.
    ///
    /// Only the two tiles either side of that divider move; the rest keep the
    /// space they had, which is what makes dragging one divider in a three-tile
    /// row feel local rather than like a reflow.
    static func dragged(
        _ base: [Double], index: Int, delta: Double, floorFraction: Double
    ) -> [Double] {
        guard index >= 0, index + 1 < base.count else { return base }
        let room = base[index] + base[index + 1]
        // Two tiles that cannot both clear the minimum are left alone rather
        // than squeezed to something arbitrary.
        guard room >= floorFraction * 2 else { return base }
        var next = base
        next[index] = max(floorFraction, min(room - floorFraction, base[index] + delta))
        next[index + 1] = room - next[index]
        return next
    }
}

enum TileLayout {
    /// An agent on the left, the branch diff on the right.
    ///
    /// The default rather than something you arrange, because the diff beside
    /// the agent changing it is the most valuable adjacency in the product and
    /// nobody should have to discover it.
    static let `default`: TileNode = .split(.horizontal, [.leaf(.tmux), .leaf(.diff)], [0.5, 0.5])

    private static func key(_ workspace: String) -> String { "tiles.layout.\(workspace)" }

    /// Client-local, per worktree, per device.
    ///
    /// Not daemon-owned: this is view arrangement rather than intent about the
    /// work, and a phone shows one tile at a time regardless, so syncing it
    /// would buy almost nothing for the cost of a protocol surface, storage and
    /// a cross-device conflict rule.
    ///
    /// A layout that fails to decode falls back to the default, which is the
    /// safe direction — a build that introduced a tile kind an older build
    /// cannot read should show the default, not nothing.
    static func load(workspace: String) -> TileNode {
        guard let data = UserDefaults.standard.data(forKey: key(workspace)),
            let node = try? JSONDecoder().decode(TileNode.self, from: data)
        else { return `default` }
        return node
    }

    static func save(_ node: TileNode, workspace: String) {
        guard let data = try? JSONEncoder().encode(node) else { return }
        UserDefaults.standard.set(data, forKey: key(workspace))
    }

    static func forget(workspace: String) {
        UserDefaults.standard.removeObject(forKey: key(workspace))
    }
}
