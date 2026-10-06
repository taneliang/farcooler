import Foundation

/// The phone's `phone.stack` as a destination, read once when the phone
/// moves to `nav.destination.v1` (ov-182 step 5), then removed.
///
/// The deepest screen is the place; a worktree takes its workspace from the
/// screen under it on the same runner, and its pane from a landing on a
/// terminal. The phone's runner is its `Host.id`, so it's the `host`.
/// Nil for an empty stack, which was Needs You and needs no migrating.
extension Destination {
    init?(phoneStack stack: [PhoneRoute]) {
        guard let top = stack.last else { return nil }
        switch top {
        case .workspace(let place):
            self.init(runner: Runner(host: place.runner), place: .workspace(place.workspace))
        case .task(let place, let task):
            self.init(runner: Runner(host: place.runner), place: .task(workspace: place.workspace, task: TaskRef(id: task)))
        case .history(let place, let status):
            self.init(runner: Runner(host: place.runner), place: .history(workspace: place.workspace, status: status))
        case .plan(let place, _), .tree(let place, _):
            // A plan page or a tree level isn't a place a relaunch reopens;
            // its workspace is.
            self.init(runner: Runner(host: place.runner), place: .workspace(place.workspace))
        case .worktree(let runner, let worktree, let landing):
            let workspace = stack.dropLast().reversed().lazy.compactMap { route -> String? in
                switch route {
                case .workspace(let place), .task(let place, _), .history(let place, _), .plan(let place, _),
                    .tree(let place, _):
                    place.runner == runner ? place.workspace : nil
                case .worktree:
                    nil
                }
            }.first
            var pane: String?
            if case .terminal(let id) = landing { pane = id }
            self.init(runner: Runner(host: runner), place: .worktree(worktree, workspace: workspace), pane: pane)
        }
    }
}
