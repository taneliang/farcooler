import AgentKit
import Foundation
import SwiftUI

/// A close that is waiting for a yes: which terminal, and what to ask.
struct CloseTerminalPending: Identifiable {
    let worktree: Worktree
    let terminal: Terminal
    let question: CloseTerminalGuard.Question

    var id: String { terminal.id }
}

/// When closing a terminal on the Mac asks first, and what it says (ov-161).
///
/// ⌘W stopped and removed the selected terminal at once, whatever it was in the
/// middle of; the phones ask through `ShellClose`. A close has to stop the
/// pane, and the pane's conversation goes with it, with no undo. The rule here
/// is the one place a sentence is told apart from a keystroke: a terminal whose
/// agent is working or waiting on you asks, and everything else (a shell, an
/// agent with nothing in flight, a pane that has already exited) closes
/// directly, because asking about the harmless case is a tax that teaches
/// people to click through the one that matters.
///
/// The sentences are `ShellClose`'s, the phones' own; only the button and the
/// last clause are the Mac's, which has terminals and not tabs.
enum CloseTerminalGuard {
    struct Question: Equatable {
        var title: String
        var message: String
    }

    /// The confirming button: a verb that says what happens, title case.
    static let confirm = "Stop Agent and Close"

    /// What to ask about closing `terminal`, or nil to close it now.
    static func question(for terminal: Terminal, at now: Date) -> Question? {
        guard [.running, .starting].contains(StateKind.parse(terminal.state)),
            terminal.agent == .working || terminal.agent == .blocked
        else { return nil }
        let running = ShellClose.running(
            agent: Terminal.name(of: terminal.preset),
            doing: ShellClose.doing(activity: terminal.activity),
            elapsed: terminal.displayDuration(at: now))
        return Question(
            title: "Close “\(terminal.label)”?",
            message: "\(running) Closing stops it and removes the terminal. You can’t undo this action.")
    }
}

extension View {
    /// The dialog a close waits behind, and what the confirming button does.
    func confirmingClose(
        _ pending: Binding<CloseTerminalPending?>, perform: @escaping (CloseTerminalPending) -> Void
    ) -> some View {
        confirmationDialog(
            pending.wrappedValue?.question.title ?? "",
            isPresented: Binding(
                get: { pending.wrappedValue != nil },
                set: { if !$0 { pending.wrappedValue = nil } }),
            presenting: pending.wrappedValue
        ) { item in
            Button(CloseTerminalGuard.confirm, role: .destructive) { perform(item) }
            Button("Cancel", role: .cancel) {}
        } message: { item in
            Text(item.question.message)
        }
    }
}
