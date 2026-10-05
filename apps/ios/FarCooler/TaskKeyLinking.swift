import SwiftUI

// Task keys in a runner's text as links (ov-196): which keys may link, from
// the boards this connection has read, and opening one as a pane's task chip
// does, by pushing the task. The rule for which words are keys is AgentKit's
// `TaskKeyLinks`.

extension Connection {
    /// What "ov-190" in this runner's text links to, and where it goes:
    /// the task, pushed over the screen showing. Links nothing without a
    /// navigator to push with, or before the runner is known.
    func taskKeyLinker(_ navigator: PhoneNavigator?) -> TaskKeyLinker {
        var open: (@MainActor (PhoneRoute) -> Void)?
        if let navigator { open = { navigator.open($0) } }
        // Each key's card (ov-299), from the boards and the plans read.
        let runner = hostId?.uuidString
        let cards = taskKeyCards.cards(
            runner: runner ?? "", boards: boards, plans: plans.states.compactMapValues(\.plan))
        // `TaskKeyLinker.phone` holds the wiring, where `swift test` reaches it.
        return .phone(runner: runner, workspaces: fleet.workspaces ?? [], boards: boards, cards: cards, open: open)
    }
}
