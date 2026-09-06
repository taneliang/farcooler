import AgentKit
import SwiftUI

/// Dragging a worktree row into a new place in the sidebar.
///
/// The sibling of `PaneDrag`, and it is a separate thing on purpose: that one
/// moves a terminal into a tmux split and this one moves a card in a list.
/// Sharing one drag would mean a row that cannot tell whether the thing over it
/// wants to be tiled beside a pane or filed above a worktree.
///
/// The payload is a workspace id that never leaves this process, held here
/// rather than round-tripped through an `NSItemProvider` for the reason
/// `PaneDrag` gives: reading an id back out of a provider is asynchronous, and a
/// drop handler that has to await its own argument cannot answer "which side did
/// this land on" in the same breath. The provider still exists, because it is
/// what makes the system start a drag at all.
///
/// The completed drop is published rather than handed to a closure, and that is
/// the one place this differs from `PaneDrag`. `WorkspaceSection` already takes
/// eighteen arguments — its own doc comment records that adding a nineteenth
/// pushed the enclosing expression past "unable to type-check in reasonable
/// time" — so the row says only that a drop happened, and `ContentView`, which
/// is the only thing that can see a whole project group and the runner it is on,
/// decides what that means.
@MainActor
final class WorkspaceDrag: ObservableObject {
    static let shared = WorkspaceDrag()

    /// The card in flight.
    @Published private(set) var dragged: String?
    /// Where it would land, as an insertion line drawn on one row's edge.
    @Published private(set) var landing: Landing?
    /// The last drop that completed. `ContentView` watches this.
    @Published private(set) var completion: Completion?

    struct Landing: Equatable {
        var workspace: String
        var edge: WorkspaceOrder.Edge
    }

    /// A finished drop, and nothing about what it means.
    ///
    /// `token` is here because two identical drops in a row are two events and
    /// `@Published` on an `Equatable` value is not: dragging a card down one
    /// place, putting it back, and doing it again would otherwise deliver two
    /// notifications and not three.
    struct Completion: Equatable {
        var dragged: String
        var target: String
        var edge: WorkspaceOrder.Edge
        var token: Int
    }

    private var token = 0

    func begin(_ workspace: String) {
        dragged = workspace
        landing = nil
    }

    /// Hovering over a row's half. Refused when nothing is being dragged, and
    /// for the dragged row itself — a card dropped on itself means nothing.
    func hover(_ workspace: String, _ edge: WorkspaceOrder.Edge) {
        guard let dragged, dragged != workspace else { return }
        let next = Landing(workspace: workspace, edge: edge)
        if landing != next { landing = next }
    }

    func leave(_ workspace: String) {
        if landing?.workspace == workspace { landing = nil }
    }

    /// End the drag with a landing, and say so.
    ///
    /// Returns false, changing nothing, when there was no drag or it landed on
    /// its own row — which is what a late update from a finished drag looks
    /// like, and what `PaneDrag` guards against for the same reason.
    @discardableResult
    func drop(on workspace: String, _ edge: WorkspaceOrder.Edge) -> Bool {
        guard let moving = dragged, moving != workspace else {
            cancel()
            return false
        }
        cancel()
        token += 1
        completion = Completion(
            dragged: moving, target: workspace, edge: edge, token: token)
        return true
    }

    func cancel() {
        dragged = nil
        landing = nil
    }

    /// The edge an insertion line would be drawn on for this row, or nil if it
    /// is not the target.
    func landing(on workspace: String) -> WorkspaceOrder.Edge? {
        landing?.workspace == workspace ? landing?.edge : nil
    }
}

/// A worktree row as a drop target.
///
/// A `DropDelegate` rather than the closure form of `.onDrop`, for the reason
/// `PaneDropTarget` gives: only a delegate is told where the pointer is while
/// the drag is still in progress, and above-or-below is entirely a question
/// about where the pointer is.
struct WorkspaceDropTarget: DropDelegate {
    let workspace: String
    /// The row's own height, measured by the row. Zero until it has been laid
    /// out, which `WorkspaceOrder.edge` reads as `.above` — the answer that
    /// cannot move a card somewhere it was not dragged.
    let height: CGFloat

    private var dragged: String? {
        guard let id = WorkspaceDrag.shared.dragged, id != workspace else { return nil }
        return id
    }

    private func edge(_ info: DropInfo) -> WorkspaceOrder.Edge {
        WorkspaceOrder.edge(pointerY: info.location.y, rowHeight: height)
    }

    func validateDrop(info: DropInfo) -> Bool { dragged != nil }

    func dropEntered(info: DropInfo) {
        WorkspaceDrag.shared.hover(workspace, edge(info))
    }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        WorkspaceDrag.shared.hover(workspace, edge(info))
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) {
        WorkspaceDrag.shared.leave(workspace)
    }

    func performDrop(info: DropInfo) -> Bool {
        WorkspaceDrag.shared.drop(on: workspace, edge(info))
    }
}
