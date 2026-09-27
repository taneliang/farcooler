import AgentKit
import SwiftUI

/// Dragging a worktree row into a new place in the sidebar.
///
/// The sibling of `PaneDrag`, and it is a separate thing on purpose: that one
/// moves a terminal into a tmux split and this one moves a card in a list.
/// Sharing one drag would mean a row that cannot tell whether the thing over it
/// wants to be tiled beside a pane or filed above a worktree.
///
/// The payload is a worktree id that never leaves this process, held here
/// rather than round-tripped through an `NSItemProvider` for the reason
/// `PaneDrag` gives: reading an id back out of a provider is asynchronous, and a
/// drop handler that has to await its own argument cannot answer "which side did
/// this land on" in the same breath. The provider still exists, because it is
/// what makes the system start a drag at all.
///
/// The completed drop is published rather than handed to a closure, and that is
/// the one place this differs from `PaneDrag`. `WorktreeSection` already takes
/// eighteen arguments — its own doc comment records that adding a nineteenth
/// pushed the enclosing expression past "unable to type-check in reasonable
/// time" — so the row says only that a drop happened, and `ContentView`, which
/// is the only thing that can see a whole project group and the runner it is on,
/// decides what that means.
@MainActor
final class WorktreeDrag: ObservableObject {
    static let shared = WorktreeDrag()

    /// The card in flight.
    @Published private(set) var dragged: String?
    /// Where it would land, as an insertion line drawn on one row's edge.
    @Published private(set) var landing: Landing?
    /// The workspace header the card is over, lit as the place it would go.
    @Published private(set) var workspaceLanding: String?
    /// The last drop that completed. `ContentView` watches this.
    @Published private(set) var completion: Completion?

    struct Landing: Equatable {
        var worktree: String
        var edge: WorktreeOrder.Edge
    }

    /// Where a card can be dropped: on one edge of another worktree's row,
    /// or on a workspace's header.
    enum Target: Equatable {
        case worktree(String, WorktreeOrder.Edge)
        case workspace(String)
    }

    /// A finished drop, and nothing about what it means.
    ///
    /// `token` is here because two identical drops in a row are two events and
    /// `@Published` on an `Equatable` value is not: dragging a card down one
    /// place, putting it back, and doing it again would otherwise deliver two
    /// notifications and not three.
    struct Completion: Equatable {
        var dragged: String
        var target: Target
        var token: Int
    }

    /// Whether a card may land on a target at all: `ContentView`'s rule
    /// (`ContentView.dropMeaning`), which only it can answer, because only it
    /// sees the fleet. Asked while hovering, so a place the drop would be
    /// refused draws no insertion line and takes no drop, rather than taking
    /// one that then does nothing. Nil lets every landing through.
    var accepts: ((_ dragged: String, _ target: Target) -> Bool)?

    private var token = 0

    /// Whether the card in flight may land on `target`.
    func allows(_ target: Target) -> Bool {
        guard let dragged else { return false }
        if case .worktree(let id, _) = target, id == dragged { return false }
        return accepts?(dragged, target) ?? true
    }

    /// Whether a worktree row can be picked up at all.
    ///
    /// Two conditions, and a row that fails either offers no drag, rather
    /// than a drag that does nothing when it lands:
    ///
    /// - The runner can take a write right now (`usable`). One already known
    ///   to be unreachable would take the request nowhere.
    /// - The runner keeps an order (`DaemonBuild.keepsWorktreeOrder`). One
    ///   too old to store a rank answers `worktree reorder` with "unknown
    ///   method", and the row springs back on the next read with nothing said
    ///   anywhere. A runner whose build has not been read yet is refused, not
    ///   guessed at: the read lands within a round trip of every link coming
    ///   up, so the handle appears a moment late rather than a drag failing.
    ///
    /// Static and free of the view so it can be tested; `WorktreeSection`
    /// holds only the answer. The phone asks the same capability through
    /// `ShellRunnerLabel.keepsOrder(daemon:)`.
    nonisolated static func offersDrag(usable: Bool, runner: DaemonBuild?) -> Bool {
        usable && (runner?.keepsWorktreeOrder ?? false)
    }

    func begin(_ worktree: String) {
        dragged = worktree
        landing = nil
        workspaceLanding = nil
    }

    /// Hovering over a row's half. Refused when nothing is being dragged, and
    /// for the dragged row itself — a card dropped on itself means nothing.
    func hover(_ worktree: String, _ edge: WorktreeOrder.Edge) {
        guard allows(.worktree(worktree, edge)) else {
            leave(worktree)
            return
        }
        let next = Landing(worktree: worktree, edge: edge)
        if landing != next { landing = next }
    }

    func leave(_ worktree: String) {
        if landing?.worktree == worktree { landing = nil }
    }

    /// Hovering over a workspace's header. Refused as `hover` is.
    func hover(workspace: String) {
        guard allows(.workspace(workspace)) else { return }
        if workspaceLanding != workspace { workspaceLanding = workspace }
    }

    func leave(workspace: String) {
        if workspaceLanding == workspace { workspaceLanding = nil }
    }

    /// End the drag with a landing, and say so.
    ///
    /// Returns false, changing nothing, when there was no drag or it landed on
    /// its own row — which is what a late update from a finished drag looks
    /// like, and what `PaneDrag` guards against for the same reason — and
    /// for a target `accepts` refuses, since a drop the sidebar would only
    /// ignore is not a drop.
    @discardableResult
    func drop(on target: Target) -> Bool {
        guard let moving = dragged, allows(target) else {
            cancel()
            return false
        }
        cancel()
        token += 1
        completion = Completion(dragged: moving, target: target, token: token)
        return true
    }

    @discardableResult
    func drop(on worktree: String, _ edge: WorktreeOrder.Edge) -> Bool {
        drop(on: .worktree(worktree, edge))
    }

    func cancel() {
        dragged = nil
        landing = nil
        workspaceLanding = nil
    }

    /// The edge an insertion line would be drawn on for this row, or nil if it
    /// is not the target.
    func landing(on worktree: String) -> WorktreeOrder.Edge? {
        landing?.worktree == worktree ? landing?.edge : nil
    }
}

/// A worktree row as a drop target.
///
/// A `DropDelegate` rather than the closure form of `.onDrop`, for the reason
/// `PaneDropTarget` gives: only a delegate is told where the pointer is while
/// the drag is still in progress, and above-or-below is entirely a question
/// about where the pointer is.
struct WorktreeDropTarget: DropDelegate {
    let worktree: String
    /// The row's own height, measured by the row. Zero until it has been laid
    /// out, which `WorktreeOrder.edge` reads as `.above` — the answer that
    /// cannot move a card somewhere it was not dragged.
    let height: CGFloat

    private var dragged: String? {
        guard let id = WorktreeDrag.shared.dragged, id != worktree else { return nil }
        return id
    }

    private func edge(_ info: DropInfo) -> WorktreeOrder.Edge {
        WorktreeOrder.edge(pointerY: info.location.y, rowHeight: height)
    }

    func validateDrop(info: DropInfo) -> Bool {
        dragged != nil && WorktreeDrag.shared.allows(.worktree(worktree, edge(info)))
    }

    func dropEntered(info: DropInfo) {
        WorktreeDrag.shared.hover(worktree, edge(info))
    }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        WorktreeDrag.shared.hover(worktree, edge(info))
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) {
        WorktreeDrag.shared.leave(worktree)
    }

    func performDrop(info: DropInfo) -> Bool {
        WorktreeDrag.shared.drop(on: worktree, edge(info))
    }
}

/// A workspace's header as a drop target: a worktree dropped on it moves to
/// that workspace (`farcooler worktree assign`). The header's whole row, with
/// no edge, because the workspace decides where among its rows the worktree
/// goes — the runner's order, as for every row.
struct WorkspaceDropTarget: DropDelegate {
    let workspace: String

    func validateDrop(info: DropInfo) -> Bool {
        WorktreeDrag.shared.allows(.workspace(workspace))
    }

    func dropEntered(info: DropInfo) {
        WorktreeDrag.shared.hover(workspace: workspace)
    }

    func dropUpdated(info: DropInfo) -> DropProposal? {
        WorktreeDrag.shared.hover(workspace: workspace)
        return DropProposal(operation: .move)
    }

    func dropExited(info: DropInfo) {
        WorktreeDrag.shared.leave(workspace: workspace)
    }

    func performDrop(info: DropInfo) -> Bool {
        WorktreeDrag.shared.drop(on: .workspace(workspace))
    }
}

/// A worktree row as a drag source, or not one at all.
///
/// A branch rather than an `.onDrag` that hands back an empty provider when it
/// should refuse, which was the old refusal for an unreachable runner. Whether
/// the system still lifts a drag image off the row for an empty provider is
/// AppKit's decision, not this file's; a row that is not a drag source at all
/// leaves nothing to decide.
///
/// The branch changes the row's identity only when its runner's answer changes,
/// which is once per link at most: when its build is first read, or when it
/// goes unreachable and comes back.
struct WorktreeDragSource: ViewModifier {
    let worktree: String
    let enabled: Bool

    func body(content: Content) -> some View {
        if enabled {
            content.onDrag {
                MainActor.assumeIsolated { WorktreeDrag.shared.begin(worktree) }
                // Carries the id only so the system will start a drag at all;
                // the payload that is actually read is in `WorktreeDrag`. See
                // its docs.
                return NSItemProvider(object: worktree as NSString)
            }
        } else {
            content
        }
    }
}
