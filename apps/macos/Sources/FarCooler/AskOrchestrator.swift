import AgentKit
import SwiftUI

/// Ask the Orchestrator (ov-184): the one thing a task offers for changing it.
///
/// The orchestrator owns the task list, so a person who wants a task moved,
/// reworded, started or dropped says so to the orchestrator. This puts the
/// start of that message in its composer, naming the task, and goes there;
/// the person finishes the sentence and presses Return.
///
/// It never starts an orchestrator. A workspace with none running (including
/// Main, which can't have one) leaves the item disabled and saying what to do,
/// rather than hidden, so a person looking for it learns why it can't be used.
enum AskOrchestrator {
    /// Why the item is off, and what turns it on.
    static let unavailable = "Start an orchestrator to ask about this task"

    /// The words left in the composer: a reference, not a copy, because the
    /// orchestrator reads the task itself (`farcooler task show`). Ends where
    /// the person goes on typing.
    static func draft(for row: TaskRow) -> String {
        "About \(row.key) (“\(row.title)”): "
    }

    /// Leave `row`'s draft in `orchestrator`'s composer. False when there's no
    /// orchestrator to leave it with, and then nothing is left anywhere.
    @MainActor
    @discardableResult
    static func ask(about row: TaskRow, of orchestrator: BoardPane?, handoff: ComposerHandoff = .shared) -> Bool {
        guard let orchestrator else { return false }
        handoff.offer(draft(for: row), to: orchestrator.terminal.short)
        return true
    }

    /// What a view needs to draw the item: whether it works now, and what it
    /// does. Handed down rather than looked up in the view, so the views
    /// redraw when an orchestrator starts or stops.
    struct Action {
        var available = false
        var perform: (TaskRow) -> Void = { _ in }

        static let unavailable = Action()
    }
}

/// The menu item and button both: disabled with the sentence, never hidden.
struct AskOrchestratorButton: View {
    let row: TaskRow
    let action: AskOrchestrator.Action
    var small = false

    var body: some View {
        Button("Ask the Orchestrator") { action.perform(row) }
            .controlSize(small ? .small : .regular)
            .disabled(!action.available)
            .help(action.available ? "Start a message to the orchestrator about this task" : AskOrchestrator.unavailable)
            .accessibilityHint(action.available ? "" : AskOrchestrator.unavailable)
            .accessibilityIdentifier("ask-orchestrator")
    }
}
