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
        case .split(let axis, let children, _):
            let parts = children.indices.map { childBinding(node, $0) }
            if axis == .horizontal {
                return AnyView(
                    HSplitView {
                        ForEach(parts.indices, id: \.self) { i in render(parts[i]) }
                    })
            }
            return AnyView(
                VSplitView {
                    ForEach(parts.indices, id: \.self) { i in render(parts[i]) }
                })
        }
    }

    /// A binding to one child of a split.
    ///
    /// Written out rather than reached for with a subscript because `TileNode`
    /// is an enum: there is no stored property to project, so the setter has to
    /// rebuild the parent case around the new child.
    private func childBinding(_ parent: Binding<TileNode>, _ index: Int) -> Binding<TileNode> {
        Binding(
            get: {
                guard case .split(_, let children, _) = parent.wrappedValue,
                    index < children.count
                else { return .leaf(.diff) }
                return children[index]
            },
            set: { newValue in
                guard case .split(let axis, var children, let fractions) = parent.wrappedValue,
                    index < children.count
                else { return }
                children[index] = newValue
                parent.wrappedValue = .split(axis, children, fractions)
            }
        )
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
