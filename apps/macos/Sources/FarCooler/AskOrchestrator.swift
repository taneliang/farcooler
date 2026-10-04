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
        "About \(oneLine(row.key)) (“\(oneLine(row.title))”): "
    }

    /// `raw` as one line that does nothing when it reaches a composer or a
    /// terminal, the way the daemon's `one_line` makes one (here by removing
    /// rather than spelling out): escape sequences (a CSI such as `ESC[201~`,
    /// which would close a bracketed paste, an OSC, or ESC and the character
    /// after it), every other control character (^C, ^D, ^U, DEL, C1), line
    /// breaks and tabs as spaces, and invisible format characters (bidi
    /// overrides, zero-width marks). Runs of space become one. The one place
    /// a task's words are made safe, for the composer, the daemon and the
    /// clipboard alike.
    static func oneLine(_ raw: String) -> String {
        var out = String.UnicodeScalarView()
        var scalars = raw.unicodeScalars[...]
        while let c = scalars.popFirst() {
            switch c.value {
            case 0x1B:
                guard let kind = scalars.popFirst() else { break }
                if kind == "[" {
                    // CSI: parameters, then one final byte, 0x40 to 0x7E.
                    while let next = scalars.popFirst(), !(0x40...0x7E).contains(next.value) {}
                } else if kind == "]" {
                    // OSC: up to BEL, or ESC and a backslash.
                    while let next = scalars.popFirst() {
                        if next.value == 0x07 { break }
                        if next.value == 0x1B { _ = scalars.popFirst(); break }
                    }
                }
            case 0x09, 0x0A, 0x0D, 0x0B, 0x0C, 0x85, 0x2028, 0x2029:
                out.append(" ")
            case 0x00...0x1F, 0x7F...0x9F, 0xAD, 0x200B...0x200F, 0x202A...0x202E, 0x2060...0x2064, 0x2066...0x2069,
                0xFEFF:
                break
            default:
                out.append(c)
            }
        }
        return String(out).split(separator: " ").joined(separator: " ")
    }

    /// Where a copy goes when the runner's client is gone: the general
    /// pasteboard, so the notice that follows is true.
    @MainActor
    static func copyToPasteboard(_ text: String) {
        NSPasteboard.general.clearContents()
        NSPasteboard.general.setString(text, forType: .string)
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

        static var unavailable: Action { Action() }
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
