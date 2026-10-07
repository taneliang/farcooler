import AgentKit
import Foundation
import SwiftUI

/// Where the native composer's message goes (ov-372): `terminal.compose`,
/// which types it into the TUI's box on one line and presses Enter past the
/// same gate as `terminal tell`, or refuses with a word and types nothing.
/// True when the agent was working and claude's own queue took it (R-29).
protocol ComposeSink: Sendable {
    func compose(terminal: String, text: String) async throws -> Bool
}

extension RunnerCore: ComposeSink {
    func compose(terminal: String, text: String) async throws -> Bool {
        let data = try await call("terminal.compose", ["terminal": terminal, "text": text])
        let object = (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
        return object["queued"] as? Bool ?? false
    }
}

/// One terminal-mode claude pane's native side: its rows, the composer's
/// draft, and which of the two views is showing (ov-372).
///
/// Held by `NativeAgents` for the life of the app rather than by a view, so
/// a layout change, a tile move or switching views never loses the draft or
/// starts the rows over. Nothing here touches the pane's process: the
/// terminal under the native view is the same tmux pane, never respawned.
@MainActor
final class NativePaneModel: ObservableObject {
    let terminal: String
    let store: AgentRowStore

    /// The composer's text. Line breaks become spaces as they arrive: the
    /// box takes one line from here until compose_into (ov-367), so what you
    /// see is what's sent.
    @Published var draft = "" {
        didSet {
            if draft.contains(where: \.isNewline) {
                draft = draft.replacingOccurrences(of: "\r\n", with: " ").replacingOccurrences(of: "\n", with: " ")
                    .replacingOccurrences(of: "\r", with: " ")
            }
        }
    }
    @Published private(set) var sending = false
    /// What stopped the last send, until the next one or a dismissal.
    @Published var issue: SendIssue?
    /// Messages claude's queue took that its transcript hasn't shown yet,
    /// drawn as Queued rows below the list.
    @Published private(set) var queued: [String] = []
    /// Whether this pane shows the native view; remembered per pane.
    @Published var showsNative: Bool {
        didSet { NativePaneModel.remember(showsNative, for: terminal) }
    }

    /// Where sends go: the runner's connection, replaced on a reconnect.
    var sink: (any ComposeSink)?
    /// Where older pages come from, once the pane is following.
    var source: (any AgentRowSource)?

    init(terminal: String, store: AgentRowStore, sink: (any ComposeSink)?) {
        self.terminal = terminal
        self.store = store
        self.sink = sink
        showsNative = NativePaneModel.remembered(for: terminal)
    }

    /// The longest message the box takes from here (`tell.rs`'s
    /// `LONGEST_MESSAGE`).
    static let longest = 500

    /// Why a message wasn't sent, as the composer says it.
    enum SendIssue: Equatable {
        /// Claude is showing a question, a menu or a panel only the terminal
        /// can draw: the Handoff row, with Show Terminal.
        case handoff
        /// The terminal's box holds text of its own (R-28).
        case draftInTerminal
        /// Something only words can say.
        case said(String)
    }

    var canSend: Bool {
        let text = draft.trimmingCharacters(in: .whitespaces)
        return !sending && !text.isEmpty && text.count <= Self.longest && sink != nil
    }

    /// Send the draft. Enter's action.
    func send() async {
        let text = draft.trimmingCharacters(in: .whitespaces)
        guard canSend, let sink else {
            if draft.trimmingCharacters(in: .whitespaces).count > Self.longest { issue = .said(Self.tooLong) }
            return
        }
        // A slash or a bang opens claude's command picker or its shell, which
        // Enter would then run: commands go through the terminal until the
        // picker is driven from here (ov-367).
        if let first = text.first, "/!#@&$?\\".contains(first) {
            issue = .said(Self.command)
            return
        }
        sending = true
        issue = nil
        defer { sending = false }
        do {
            let wasQueued = try await sink.compose(terminal: terminal, text: text)
            if draft.trimmingCharacters(in: .whitespaces) == text { draft = "" }
            if wasQueued { queued.append(text) }
        } catch {
            issue = Self.issue(for: error)
        }
    }

    /// The page above the oldest row held.
    func loadOlder() {
        if let source { store.loadOlder(source) }
    }

    /// Drop a local Queued echo once the transcript shows the message, as a
    /// Queued row or as the turn it became.
    func settleQueued() {
        guard !queued.isEmpty else { return }
        let shown = Set(store.ids.suffix(40).compactMap { store.box($0)?.row }.compactMap { row -> String? in
            switch row.kind {
            case .queued(let q): q.text
            case .turn(let t): t.prompt
            default: nil
            }
        })
        queued.removeAll { shown.contains($0) }
    }

    static func issue(for error: Error) -> SendIssue {
        let failure = error as? RunnerCore.Failure
        switch failure?.what {
        case "prompt", "dialog": return .handoff
        case "draft": return .draftInTerminal
        case "typing": return .said("Someone is typing in the terminal. Try again in a moment.")
        case "busy": return .said("Claude is working and can’t take a message from here right now.")
        case "too_long": return .said(tooLong)
        case "command": return .said(command)
        case "paste_left": return .said("The message didn’t land in the box as typed, so it was left there and not sent.")
        case "left_at_shell": return .said("Claude quit as the message was typed. It wasn’t run.")
        case "unconfirmed": return .said("Claude didn’t confirm it queued the message. Check the terminal before sending it again.")
        case "not_running", "not_an_agent": return .said("Claude isn’t running in this pane.")
        case "unfamiliar", "unproven": return .said("Far Cooler can’t read this terminal’s box, so nothing was typed.")
        default:
            if case .lost = failure { return .said("The runner didn’t answer. The message may not have been sent.") }
            return .said("The message wasn’t sent.")
        }
    }

    static let tooLong = "That message is over \(longest) characters. Shorten it, or paste it in the terminal."
    static let command = "Messages can’t start with / or !, which Claude reads as a command. Use the terminal for commands."

    // MARK: - The view each pane remembers (R-27)

    private static let key = "nativeAgent.paneViews"

    /// Terminal on the Mac until a pane was switched (R-27).
    static func remembered(for terminal: String, defaults: UserDefaults = .standard) -> Bool {
        (defaults.dictionary(forKey: key) as? [String: Bool])?[terminal] ?? false
    }

    static func remember(_ native: Bool, for terminal: String, defaults: UserDefaults = .standard) {
        var all = (defaults.dictionary(forKey: key) as? [String: Bool]) ?? [:]
        all[terminal] = native ? true : nil
        defaults.set(all, forKey: key)
    }
}
