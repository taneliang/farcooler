import AppKit
import SwiftUI

/// Renders a worktree's tile tree.
///
/// Recursive and deliberately dumb: it draws splits and hands each leaf to
/// whatever owns that tile kind. The tmux leaf comes back to the caller as a
/// closure, because the pane canvas needs the whole of `ContentView`'s state and
/// this view has no business knowing about it.
struct TileContainer: View {
    @Binding var node: TileNode
    let workspace: Workspace
    @ObservedObject var changes: ChangesStore
    let tmux: () -> AnyView

    var body: some View {
        render($node)
    }

    /// The tree, one node at a time.
    ///
    /// Children are wrapped in `AnyView` deliberately. A recursive SwiftUI view
    /// cannot infer `some View` from itself — the opaque type would be defined
    /// in terms of its own body — so the recursion has to pass through an erased
    /// type. The cost is a box per split, on a tree with two or three leaves.
    private func render(_ node: Binding<TileNode>) -> AnyView {
        switch node.wrappedValue {
        case .leaf(let kind):
            return AnyView(leaf(kind))
        case .split:
            return AnyView(TileSplit(node: node, render: render))
        }
    }

    @ViewBuilder
    private func leaf(_ kind: TileKind) -> some View {
        switch kind {
        case .tmux:
            tmux()
        case .diff:
            DiffTile(changes: changes)
        case .pr:
            // Saying so beats an empty pane, which would read as a worktree with
            // no pull request rather than a tile that has not been built.
            TilePlaceholder(
                title: "Pull requests aren't here yet",
                detail: "This tile arrives with the PR work.")
        }
    }
}

/// One split, sized by the fractions its node carries.
///
/// An `HSplitView` was the first attempt and ignored those fractions entirely.
/// It sizes children by what they ask for, the tmux canvas asks for a lot, and
/// the diff tile arrived as a sliver with only its scope picker showing — a
/// stored layout that the layout engine quietly overruled. Sizes are computed
/// here instead, which is also what makes a dragged divider something that can
/// be written down.
private struct TileSplit: View {
    @Binding var node: TileNode
    let render: (Binding<TileNode>) -> AnyView

    /// The fractions as they were when the current drag began.
    ///
    /// Held because a drag reports its translation from where it started, so
    /// applying each report to the latest fractions would compound them.
    @State private var start: [Double]?

    private static let dividerWidth: CGFloat = 7
    /// No tile can be dragged narrower than this.
    private static let floor: CGFloat = 140

    var body: some View {
        guard case .split(let axis, let children, let fractions) = node,
            !children.isEmpty, fractions.count == children.count
        else {
            // A malformed node draws nothing rather than crashing. It can only
            // arrive from a decoded layout an older build wrote.
            return AnyView(Color.clear)
        }

        return AnyView(
            GeometryReader { geo in
                let total = axis == .horizontal ? geo.size.width : geo.size.height
                let usable = max(
                    1, total - Self.dividerWidth * CGFloat(children.count - 1))
                let sizes = Self.sizes(fractions, usable)

                if axis == .horizontal {
                    HStack(spacing: 0) { lane(axis, children.count, sizes, usable) }
                } else {
                    VStack(spacing: 0) { lane(axis, children.count, sizes, usable) }
                }
            })
    }

    @ViewBuilder
    private func lane(
        _ axis: Axis, _ count: Int, _ sizes: [CGFloat], _ usable: CGFloat
    ) -> some View {
        ForEach(0..<count, id: \.self) { i in
            render(childBinding(i))
                .frame(
                    width: axis == .horizontal ? sizes[i] : nil,
                    height: axis == .vertical ? sizes[i] : nil)
            if i < count - 1 {
                handle(axis, i, usable)
            }
        }
    }

    private static func sizes(_ fractions: [Double], _ usable: CGFloat) -> [CGFloat] {
        TileSizing.sizes(fractions, usable: Double(usable)).map { CGFloat($0) }
    }

    private func handle(_ axis: Axis, _ index: Int, _ usable: CGFloat) -> some View {
        Divider()
            .frame(
                width: axis == .horizontal ? Self.dividerWidth : nil,
                height: axis == .vertical ? Self.dividerWidth : nil)
            // The divider line is one point; the grab area is the whole width of
            // this frame, which is why the shape is drawn rather than left to
            // the line's own bounds.
            .contentShape(Rectangle())
            .onHover { inside in
                if inside {
                    (axis == .horizontal ? NSCursor.resizeLeftRight : NSCursor.resizeUpDown)
                        .set()
                } else {
                    NSCursor.arrow.set()
                }
            }
            .gesture(
                DragGesture(minimumDistance: 1)
                    .onChanged { g in
                        guard case .split(let axis2, let children, let current) = node
                        else { return }
                        let base = start ?? current
                        if start == nil { start = current }
                        let moved =
                            axis == .horizontal ? g.translation.width : g.translation.height
                        node = .split(
                            axis2, children,
                            TileSizing.dragged(
                                base, index: index, delta: Double(moved / usable),
                                floorFraction: Double(Self.floor / usable)))
                    }
                    .onEnded { _ in start = nil }
            )
    }

    /// A binding to one child of the split.
    ///
    /// Written out rather than reached for with a subscript because `TileNode`
    /// is an enum: there is no stored property to project, so the setter has to
    /// rebuild the parent case around the new child.
    private func childBinding(_ index: Int) -> Binding<TileNode> {
        Binding(
            get: {
                guard case .split(_, let children, _) = node, index < children.count
                else { return .leaf(.diff) }
                return children[index]
            },
            set: { newValue in
                guard case .split(let axis, var children, let fractions) = node,
                    index < children.count
                else { return }
                children[index] = newValue
                node = .split(axis, children, fractions)
            }
        )
    }
}

/// A tile with nothing to show, that says which nothing it is.
struct TilePlaceholder: View {
    let title: String
    let detail: String

    var body: some View {
        VStack(spacing: 4) {
            Text(title).font(.system(size: 12, weight: .medium))
            Text(detail)
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(16)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}
