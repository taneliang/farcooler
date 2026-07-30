import SwiftUI

/// A set of terminals shown together, as the daemon holds it.
///
/// Read, never computed. The daemon owns which terminals belong on screen
/// together and in what order, because the CLI and any agent driving it change
/// the same thing — a layout the app decided for itself would be invisible to
/// both.
struct PaneGroup: Decodable, Identifiable, Hashable {
    var id: String
    var short: String?
    var name: String
    var preset: String
    var ratio: Double?
    var active: Bool?
    var zoomed: String?
    var focused: String?
    var members: [PaneMember]

    var isActive: Bool { active ?? false }
    var share: Double { ratio ?? 0.62 }
    var terminals: [String] { members.map(\.id) }
    var layout: TilePreset { TilePreset(rawValue: preset) ?? .tiled }
}

struct PaneMember: Decodable, Identifiable, Hashable {
    var id: String
    var short: String?
    var title: String?
}

struct PaneGroupList: Decodable {
    var workspace: String
    var groups: [PaneGroup]

    var active: PaneGroup? { groups.first { $0.isActive } ?? groups.first }
}

/// The five arrangements, by tmux's names.
///
/// The names are the wire format as well as the labels: a great many people
/// already know what `main-vertical` means, and a second vocabulary for the same
/// five shapes would have cost them that for nothing.
enum TilePreset: String, CaseIterable, Identifiable {
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

/// Where each pane goes.
///
/// Pure, and the only copy. The daemon owns the ordered member list and the
/// preset and deliberately owns no geometry, so this function is both what draws
/// the panes and what answers "which pane is to the left of this one" — there is
/// no second implementation to disagree with the first.
enum TileGeometry {
    /// Frames for `count` panes inside `bounds`, in member order.
    static func frames(
        count: Int,
        preset: TilePreset,
        ratio: Double,
        in bounds: CGRect,
        gap: CGFloat = 6
    ) -> [CGRect] {
        guard count > 0 else { return [] }
        guard count > 1 else { return [bounds] }

        switch preset {
        case .evenHorizontal:
            return slice(bounds, into: count, gap: gap, vertical: false)
        case .evenVertical:
            return slice(bounds, into: count, gap: gap, vertical: true)
        case .mainVertical:
            let width = (bounds.width - gap) * ratio
            let main = CGRect(x: bounds.minX, y: bounds.minY, width: width, height: bounds.height)
            let rest = CGRect(
                x: main.maxX + gap, y: bounds.minY,
                width: bounds.width - width - gap, height: bounds.height)
            return [main] + slice(rest, into: count - 1, gap: gap, vertical: true)
        case .mainHorizontal:
            let height = (bounds.height - gap) * ratio
            let main = CGRect(x: bounds.minX, y: bounds.minY, width: bounds.width, height: height)
            let rest = CGRect(
                x: bounds.minX, y: main.maxY + gap,
                width: bounds.width, height: bounds.height - height - gap)
            return [main] + slice(rest, into: count - 1, gap: gap, vertical: false)
        case .tiled:
            return grid(bounds, count: count, gap: gap)
        }
    }

    private static func slice(
        _ bounds: CGRect, into count: Int, gap: CGFloat, vertical: Bool
    ) -> [CGRect] {
        guard count > 0 else { return [] }
        let total = (vertical ? bounds.height : bounds.width) - gap * CGFloat(count - 1)
        let each = total / CGFloat(count)
        return (0..<count).map { i in
            let offset = (each + gap) * CGFloat(i)
            return vertical
                ? CGRect(x: bounds.minX, y: bounds.minY + offset, width: bounds.width, height: each)
                : CGRect(x: bounds.minX + offset, y: bounds.minY, width: each, height: bounds.height)
        }
    }

    /// tmux's `tiled`: as square as the count allows, remainder on the last row.
    private static func grid(_ bounds: CGRect, count: Int, gap: CGFloat) -> [CGRect] {
        let columns = Int(ceil(Double(count).squareRoot()))
        let rows = Int(ceil(Double(count) / Double(columns)))
        let rowHeight = (bounds.height - gap * CGFloat(rows - 1)) / CGFloat(rows)

        var out: [CGRect] = []
        var placed = 0
        for row in 0..<rows {
            // The last row takes what is left rather than leaving a hole, which
            // is what makes three panes read as one-over-two instead of as a
            // grid with a gap in it.
            let inRow = min(columns, count - placed)
            let width = (bounds.width - gap * CGFloat(inRow - 1)) / CGFloat(inRow)
            for column in 0..<inRow {
                out.append(
                    CGRect(
                        x: bounds.minX + (width + gap) * CGFloat(column),
                        y: bounds.minY + (rowHeight + gap) * CGFloat(row),
                        width: width, height: rowHeight))
            }
            placed += inRow
        }
        return out
    }

    /// The pane nearest in a direction, or nil at the edge.
    ///
    /// Geometric rather than ordinal, because `prefix ←` has to mean what it
    /// looks like: in a main-vertical layout the pane to the left of the third
    /// one is the big one, and its index is 0.
    static func neighbour(
        of index: Int, direction: TileDirection, frames: [CGRect]
    ) -> Int? {
        guard frames.indices.contains(index) else { return nil }
        let from = frames[index]
        var best: (index: Int, distance: CGFloat)?

        for (other, frame) in frames.enumerated() where other != index {
            guard direction.isAhead(frame, of: from) else { continue }
            // Distance along the axis first, then overlap across it: the nearest
            // pane in the direction you pressed, preferring one you are level
            // with over one that merely starts sooner.
            let along = direction.distance(from: from, to: frame)
            let across = direction.misalignment(from: from, to: frame)
            let score = along + across * 0.5
            if best == nil || score < best!.distance {
                best = (other, score)
            }
        }
        return best?.index
    }
}

enum TileDirection {
    case left, right, up, down

    func isAhead(_ frame: CGRect, of origin: CGRect) -> Bool {
        switch self {
        case .left: return frame.midX < origin.midX
        case .right: return frame.midX > origin.midX
        case .up: return frame.midY < origin.midY
        case .down: return frame.midY > origin.midY
        }
    }

    func distance(from origin: CGRect, to frame: CGRect) -> CGFloat {
        switch self {
        case .left, .right: return abs(frame.midX - origin.midX)
        case .up, .down: return abs(frame.midY - origin.midY)
        }
    }

    func misalignment(from origin: CGRect, to frame: CGRect) -> CGFloat {
        switch self {
        case .left, .right: return abs(frame.midY - origin.midY)
        case .up, .down: return abs(frame.midX - origin.midX)
        }
    }
}
