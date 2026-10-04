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
        // One line: a title with a line break would be a second line in the
        // composer, or an Enter in a terminal.
        let title = row.title.split(whereSeparator: \.isNewline).joined(separator: " ")
        return "About \(row.key) (“\(title)”): "
    }

    /// Leave `row`'s draft in a chat orchestrator's composer. False when
    /// there's no orchestrator to leave it with, and then nothing is left
    /// anywhere.
    @MainActor
    @discardableResult
    static func ask(about row: TaskRow, of orchestrator: BoardPane?, handoff: ComposerHandoff = .shared) -> Bool {
        guard let orchestrator else { return false }
        handoff.offer(draft(for: row), to: orchestrator.terminal.short)
        return true
    }

    /// Where the reference went.
    enum Delivery: Equatable {
        /// A chat orchestrator's composer, waiting for the person.
        case composer
        /// A terminal orchestrator's input line, pasted by the daemon with no
        /// Enter.
        case pasted
        /// Onto the clipboard: the daemon couldn't prove the pane safe.
        case copied
    }

    /// What the window says when the reference was copied instead of pasted.
    static func copiedNotice(for row: TaskRow) -> String {
        "Copied a reference to \(row.key). Paste it into the orchestrator."
    }

    /// Hand the reference to `orchestrator`, whichever kind of pane it is.
    ///
    /// A chat pane takes it in its composer. A terminal pane (the owner's
    /// shell running claude, adopted as the orchestrator) is asked of the
    /// daemon, which pastes it with no Enter only past the gate that types an
    /// answer: a proven, idle agent with an empty box and a known paste mode.
    /// Anything less, or any failure, copies it instead. Nothing here ever
    /// presses Enter, and the Mac never writes to the pane itself.
    @MainActor
    static func deliver(
        _ row: TaskRow, to orchestrator: BoardPane,
        paste: (String) async -> Bool, copy: (String) -> Void,
        handoff: ComposerHandoff = .shared
    ) async -> Delivery {
        if orchestrator.terminal.isAgentPane {
            ask(about: row, of: orchestrator, handoff: handoff)
            return .composer
        }
        if await paste(draft(for: row)) { return .pasted }
        copy(draft(for: row).trimmingCharacters(in: .whitespaces))
        return .copied
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
