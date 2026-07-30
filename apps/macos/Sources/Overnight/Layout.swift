import SwiftUI

/// One layout: a tmux window, and where tmux has put every pane in it.
///
/// The app used to hold an ordered list of members and a preset name, and work
/// out the frames itself. That was two implementations of the same idea — one in
/// Swift for the screen and one in tmux for the session — and they disagreed the
/// moment anything arbitrary happened, because a list plus a preset cannot
/// express "split the right half of the bottom pane". tmux's own model can, and
/// tmux is already running.
///
/// So this carries no arrangement of its own. `columns` × `rows` is the cell grid
/// tmux laid out into, each pane names a rectangle inside it, and drawing is a
/// scale. There is nothing here for the app to be wrong about.
struct PaneGroup: Decodable, Identifiable, Hashable, Sendable {
    /// tmux's window id, `@7`. Not a uuid: layouts are tmux's objects, and every
    /// command that takes one takes this.
    var id: String
    var name: String
    var active: Bool
    /// The window's cell grid, which is what the pane rectangles are measured in.
    var columns: Int
    var rows: Int
    /// tmux's own layout string. Opaque here, and deliberately so — it is carried
    /// for identity, because it changes exactly when the arrangement does and is
    /// therefore the right thing to animate against.
    var layout: String
    var panes: [PaneRect]

    var isActive: Bool { active }

    /// The terminals in this layout, in tmux's pane order.
    var terminals: [String] { panes.map(\.id) }

    /// Derived rather than carried, because tmux has exactly one answer and a
    /// second field would be a copy of it that could go stale.
    var focused: String? { panes.first(where: \.focused)?.id }
    var zoomed: String? { panes.first(where: \.zoomed)?.id }

    func pane(_ terminal: String) -> PaneRect? { panes.first { $0.id == terminal } }

    /// The pane nearest in a direction, or nil at the edge.
    ///
    /// Resolved from the rectangles tmux reports, which is the whole point of the
    /// new model: `⌃L` has to mean the pane you can see to the right, at any depth
    /// of nesting, and only tmux knows where that is. The previous version scored
    /// against frames the app had recomputed from a preset, so in any arrangement
    /// the preset could not describe it moved focus somewhere arbitrary.
    func neighbour(of terminal: String, _ direction: TileDirection) -> PaneRect? {
        guard let from = pane(terminal) else { return nil }
        var best: (pane: PaneRect, score: Double)?

        for other in panes where other.id != terminal {
            guard direction.isAhead(other, of: from) else { continue }
            // Distance along the axis first, then how far off-centre across it:
            // the nearest pane in the direction pressed, preferring one you are
            // level with over one that merely starts sooner.
            let score = direction.distance(from: from, to: other)
                + direction.misalignment(from: from, to: other) * 0.5
            if best == nil || score < best!.score { best = (other, score) }
        }
        return best?.pane
    }
}

/// One pane, as a rectangle of cells inside its layout.
///
/// Cells rather than points, because that is what tmux computes in and what the
/// program inside the pane is sized by. Converting to points is the view's job
/// and happens in one expression; converting the other way would be the app
/// guessing at tmux's arithmetic again.
struct PaneRect: Decodable, Identifiable, Hashable, Sendable {
    var id: String
    var short: String
    /// Absent from the pushed event form, which carries only what changed about
    /// placement. Optional rather than defaulted: Swift's synthesized `Decodable`
    /// ignores default values and would throw on the whole layout over this one
    /// field, and the pane header takes its title from the terminal record anyway.
    var title: String?
    var left: Int
    var top: Int
    var columns: Int
    var rows: Int
    var focused: Bool
    var zoomed: Bool
}

struct PaneGroupList: Decodable, Sendable {
    var workspace: String
    var groups: [PaneGroup]

    var active: PaneGroup? { groups.first { $0.isActive } ?? groups.first }
}

/// An edge of a pane.
///
/// One enum for two jobs that turned out to be the same one: which way `⌃L` moves
/// focus, and which side of a pane a dragged terminal lands on. The raw values are
/// the CLI's spellings, so a keyboard direction can be passed to `layout split`
/// or `layout move` without a translation table in between — and a translation
/// table is exactly where "up" quietly became "bottom" the last time.
enum TileDirection: String, CaseIterable, Sendable {
    case left, right, top, bottom

    /// Is `pane` on this side of `origin`?
    func isAhead(_ pane: PaneRect, of origin: PaneRect) -> Bool {
        switch self {
        case .left: return pane.midX < origin.midX
        case .right: return pane.midX > origin.midX
        case .top: return pane.midY < origin.midY
        case .bottom: return pane.midY > origin.midY
        }
    }

    func distance(from origin: PaneRect, to pane: PaneRect) -> Double {
        switch self {
        case .left, .right: return abs(pane.midX - origin.midX)
        case .top, .bottom: return abs(pane.midY - origin.midY)
        }
    }

    func misalignment(from origin: PaneRect, to pane: PaneRect) -> Double {
        switch self {
        case .left, .right: return abs(pane.midY - origin.midY)
        case .top, .bottom: return abs(pane.midX - origin.midX)
        }
    }

    /// Where a drop landed, as an edge of the pane it landed on.
    ///
    /// VS Code's rule, because it is the one everybody already has: the pane
    /// splits on the edge you dropped nearest. The middle is a dead zone that
    /// means `right` rather than "nearest edge", so a drop aimed at the centre of
    /// a pane does one predictable thing instead of flickering between four
    /// answers as the pointer shakes.
    static func drop(at point: CGPoint, in size: CGSize) -> TileDirection {
        guard size.width > 0, size.height > 0 else { return .right }
        let x = point.x / size.width
        let y = point.y / size.height

        // A fifth of the pane each way. Wide enough to be hard to miss with a
        // dragged window under the cursor, narrow enough that aiming at an edge
        // still gets that edge.
        let dead = 0.2
        if abs(x - 0.5) < dead && abs(y - 0.5) < dead { return .right }

        let edges: [(TileDirection, Double)] = [
            (.left, x), (.right, 1 - x), (.top, y), (.bottom, 1 - y),
        ]
        return edges.min { $0.1 < $1.1 }?.0 ?? .right
    }

    /// The half of a pane a drop on this edge would occupy, as an alignment and a
    /// pair of fractions. Used only to draw the indicator.
    var half: (alignment: Alignment, width: CGFloat, height: CGFloat) {
        switch self {
        case .left: return (.leading, 0.5, 1)
        case .right: return (.trailing, 0.5, 1)
        case .top: return (.top, 1, 0.5)
        case .bottom: return (.bottom, 1, 0.5)
        }
    }
}

extension PaneRect {
    var midX: Double { Double(left) + Double(columns) / 2 }
    var midY: Double { Double(top) + Double(rows) / 2 }
}

/// The five arrangements, by tmux's names.
///
/// Names only. There used to be geometry behind this enum — a `frames(count:
/// preset:ratio:in:)` that drew what it thought `main-vertical` meant — and it is
/// gone: tmux applies the preset and reports the result, so the app's copy could
/// only ever be a second opinion about someone else's window.
///
/// The names are the wire format as well as the labels, because a great many
/// people already know what `main-vertical` means and a second vocabulary for the
/// same five shapes would have cost them that for nothing.
enum TilePreset: String, CaseIterable, Identifiable, Sendable {
    case evenHorizontal = "even-horizontal"
    case evenVertical = "even-vertical"
    case mainVertical = "main-vertical"
    case mainHorizontal = "main-horizontal"
    case tiled

    var id: String { rawValue }

    var label: String {
        switch self {
        case .evenHorizontal: return "Columns"
        case .evenVertical: return "Rows"
        case .mainVertical: return "Main Left"
        case .mainHorizontal: return "Main Top"
        case .tiled: return "Grid"
        }
    }

    var symbol: String {
        switch self {
        case .evenHorizontal: return "rectangle.split.3x1"
        case .evenVertical: return "rectangle.split.1x2"
        case .mainVertical: return "sidebar.squares.right"
        case .mainHorizontal: return "rectangle.grid.1x2"
        case .tiled: return "rectangle.split.2x2"
        }
    }
}

/// The drag in progress: what is being dragged, and where it would land.
///
/// The payload is a terminal id that never leaves this process, so it is held
/// here rather than round-tripped through an `NSItemProvider`. The provider still
/// exists — it is what makes the system start a drag at all — but reading the id
/// back out of it is asynchronous, and a drop handler that has to await its own
/// argument cannot answer "which side did this land on" in the same breath.
///
/// The LANDING lives here too, and that is the more important half. It was per-
/// pane `@State` first, one flag each, and the panes disagreed: a pane could be
/// left showing the half a drop had already landed in, because clearing it
/// depended on that pane being told the drag had finished. There is only ever one
/// drag and one place it would land, so there is now one value that says so —
/// and it cannot be set at all unless a drag is actually in progress, which is
/// what makes a late update from a finished drag a no-op instead of a stain.
@MainActor
final class PaneDrag: ObservableObject {
    static let shared = PaneDrag()

    @Published private(set) var terminal: String?
    @Published private(set) var landing: Landing?

    struct Landing: Equatable {
        var pane: String
        var side: TileDirection
    }

    func begin(_ terminal: String) {
        self.terminal = terminal
        landing = nil
    }

    /// Hovering over a pane's edge. Refused when nothing is being dragged, and
    /// for the dragged pane itself — dropping a pane on itself means nothing.
    func hover(_ pane: String, _ side: TileDirection) {
        guard let terminal, terminal != pane else { return }
        let next = Landing(pane: pane, side: side)
        if landing != next { landing = next }
    }

    func leave(_ pane: String) {
        if landing?.pane == pane { landing = nil }
    }

    func end() {
        terminal = nil
        landing = nil
    }

    /// The side a drop on this pane would take, or nil if it is not the target.
    func landing(on pane: String) -> TileDirection? {
        landing?.pane == pane ? landing?.side : nil
    }
}
