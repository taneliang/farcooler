import AgentKit
import Foundation

/// Stop and Send Now (ov-368): the runner presses one Esc, or claude's
/// ctrl+x ctrl+s, in the pane's TUI, past the same gate as a send, and answers
/// once claude took it; or it refuses with a word and presses nothing.
/// `terminal.interrupt` stops the turn claude is working on;
/// `terminal.send_now` sends every message waiting in claude's queue now,
/// stopping a reply that's streaming, or handing them to a command that's
/// running.
protocol InterruptSink: Sendable {
    func interrupt(terminal: String) async throws
    func sendNow(terminal: String) async throws
}

extension RunnerCore: InterruptSink {
    func interrupt(terminal: String) async throws {
        _ = try await call("terminal.interrupt", ["terminal": terminal])
    }

    func sendNow(terminal: String) async throws {
        _ = try await call("terminal.send_now", ["terminal": terminal])
    }
}

/// Which key the runner is asked to press.
enum PaneKey: Equatable {
    case stop, sendNow
}

extension NativePaneModel {
    /// Whether claude is working on a turn, as the newest turn's row says
    /// (claude's registry, `busy`). Not while a dialog is up: claude's
    /// registry says `waiting` then, which the runner carries to the row as
    /// `Waiting`, and an Esc would answer the dialog No.
    var working: Bool {
        // The newest turn's row alone (`newestTurn`): reading each row after
        // it redrew the transcript and the composer on every streamed word,
        // 9 to 20 ms a time, while the reader scrolled (ov-691).
        AgentConversation.isWorking(newestTurn: store.newestTurn)
    }

    /// The prompt claude's own box suggests, offered as the composer's
    /// placeholder while nothing is typed (ov-409).
    var suggestion: String? { store.suggestion(draft: draft) }

    /// claude's generic `Try "…"` example, in place of "Message Claude" while
    /// nothing is typed (ov-409). A hint: Tab does not take it, as in claude.
    var hint: String? { store.hint(draft: draft) }

    /// Whether Stop is offered: the runner serves it and claude is working.
    var offersStop: Bool { keys != nil && AgentConversation.pressesKeys(preset: program) && working && !store.isStale }

    /// Whether Send Now is offered on a Queued row.
    var offersSendNow: Bool { offersStop }

    /// Stop the turn: ⌘. or the Stop button.
    func stop() async { await press(.stop) }

    /// Send what waits in claude's queue now: a Queued row's Send Now.
    func sendNow() async { await press(.sendNow) }

    private func press(_ key: PaneKey) async {
        guard let keys, pressing == nil, offersStop else { return }
        pressing = key
        defer { pressing = nil }
        issue = nil
        do {
            switch key {
            case .stop: try await keys.interrupt(terminal: terminal)
            case .sendNow: try await keys.sendNow(terminal: terminal)
            }
        } catch {
            issue = Self.keyIssue(for: error, key)
        }
    }

    /// What the composer says when the runner didn't press `key`, or nil
    /// when there's nothing to say: AgentKit's words for it (`conversation_words`).
    static func keyIssue(for error: Error, _ key: PaneKey) -> SendIssue? {
        AgentConversation.keyIssue(for: failure(error), key == .stop ? .stop : .sendNow).map(issue(from:))
    }
}
