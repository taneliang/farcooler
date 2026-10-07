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
        for id in store.ids.reversed() {
            if case .turn(let turn)? = store.box(id)?.row.kind {
                return turn.outcome == nil && turn.activity == "Busy"
            }
        }
        return false
    }

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
    /// when there's nothing to say: the turn ended on its own (`idle`), or a
    /// second press came too soon after the first (`too_soon`).
    static func keyIssue(for error: Error, _ key: PaneKey) -> SendIssue? {
        let failure = error as? RunnerCore.Failure
        let stop = key == .stop
        switch failure?.what {
        case "idle", "too_soon": return nil
        case "prompt": return .handoff
        case "draft": return .draftInTerminal
        case "typing": return .said("Someone typed in the terminal in the last 2 seconds. Try again in a moment.")
        case "sending": return .said("A message is still going in. Try again in a moment.")
        case "nothing_queued": return .said("Nothing is waiting in Claude’s queue.")
        case "settling":
            return .said(stop ? "Claude is starting a step. Try Stop again in a moment."
                : "Claude is starting a step. Try Send Now again in a moment.")
        case "unconfirmed":
            return .said(stop ? "Claude didn’t confirm it stopped. It may have stopped; check the terminal before pressing again."
                : "Claude didn’t confirm it sent the queued messages. Check the terminal.")
        default:
            switch failure {
            case .timedOut?, .lost(_, notSent: false)?:
                return .said("The runner didn’t answer in time. Check the terminal.")
            case .lost(_, notSent: true)?, .notConnected?:
                return .said("The runner isn’t connected. Use the terminal.")
            default:
                return .said(stop ? "Far Cooler can’t stop Claude safely from here. Use the terminal."
                    : "Far Cooler can’t send the queue safely from here. Use the terminal.")
            }
        }
    }
}
