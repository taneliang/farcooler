import Foundation

/// The conversation view of a terminal-mode claude pane on a phone (ov-373):
/// the rules a phone decides by, here so `swift test` reads them back. The
/// Mac's native view (ov-372) keeps its own copies in `NativePaneModel` and
/// `NativeCopy`, with the same words; a later card moves it onto these.
public enum AgentConversation {
    /// Whether a runner serves the view: rows to read and a way to send. A
    /// runner with rows from before `terminal.compose` gets the terminal, not
    /// a view whose every send would fail. `agent_rows` is offered only while
    /// the runner's projector is on, so this is also the setting.
    public static func served(by build: DaemonBuild?) -> Bool {
        guard let build else { return false }
        return build.can(.agentRows) && build.can(.agentCompose)
    }

    /// Whether a pane is claude in a terminal, the one kind of pane the
    /// runner projects rows for.
    public static func isClaudeInATerminal(paneMode: String?, preset: String) -> Bool {
        (paneMode ?? "terminal") == "terminal" && preset.hasPrefix("claude")
    }

    /// Whether the pane's process is there to talk to. A pane whose claude
    /// has exited shows its terminal, which says how it ended, rather than a
    /// conversation whose every send would be refused.
    public static func isRunning(state: String) -> Bool {
        state == "running" || state == "starting"
    }

    // MARK: - The view each pane remembers (R-27)

    /// One key per pane.
    public static func viewKey(for terminal: String) -> String { "nativeAgent.view.\(terminal)" }

    /// Whether `terminal` shows the conversation: on a phone, yes until the
    /// pane was switched to its terminal (R-27).
    public static func showsConversation(_ terminal: String, defaults: UserDefaults = .standard) -> Bool {
        defaults.object(forKey: viewKey(for: terminal)) as? Bool ?? true
    }

    public static func remember(conversation: Bool, for terminal: String, defaults: UserDefaults = .standard) {
        defaults.set(conversation, forKey: viewKey(for: terminal))
    }

    // MARK: - The composer

    /// The longest message the box takes from here (`tell.rs`'s
    /// `LONGEST_MESSAGE`).
    public static let longest = 500

    /// A draft as the box will take it: one line, since compose types one
    /// (multi-line is compose_into, ov-367), so what you see is what's sent.
    public static func flattened(_ draft: String) -> String {
        guard draft.contains(where: \.isNewline) else { return draft }
        return draft.replacingOccurrences(of: "\r\n", with: " ").replacingOccurrences(of: "\n", with: " ")
            .replacingOccurrences(of: "\r", with: " ")
    }

    /// Whether a message starts with a symbol claude reads as a command: a
    /// slash or a bang opens its picker or its shell, which Enter would then
    /// run. Commands go through the terminal until the picker is driven from
    /// here (ov-367).
    public static func isCommand(_ text: String) -> Bool {
        guard let first = text.first else { return false }
        return "/!#@&$?\\".contains(first)
    }

    /// Why a message wasn't sent, as the composer says it.
    public enum SendIssue: Equatable, Sendable {
        /// Claude is showing a question, a menu or a panel only the terminal
        /// can draw: the Handoff row, with Show Terminal.
        case handoff
        /// The terminal's box holds text of its own (R-28): refused, with
        /// Show Terminal. Bring Here is a later card.
        case draftInTerminal
        /// Something only words can say.
        case said(String)
    }

    /// How a failed send came back from the client core.
    public enum SendFailure: Equatable, Sendable {
        /// The runner refused it, naming why in `what` (`terminal.compose`'s
        /// words: `dialog`, `draft`, `busy`, …) and, for a refusal that isn't
        /// compose's own, its error word (`scope-denied`).
        case refused(what: String?, word: String? = nil)
        /// No answer by the call's deadline, or one that couldn't be read:
        /// either way it may still be typed.
        case timedOut
        /// The link dropped. `notSent` when the call provably never left
        /// this phone; otherwise it may have reached the runner.
        case lost(notSent: Bool)
    }

    public static func issue(for failure: SendFailure) -> SendIssue {
        switch failure {
        // A grant that may read but not type (`read`): saying so beats
        // "wasn't sent" on every try.
        case .refused(_, let word?) where word == "scope-denied":
            return .said("This device can’t send messages to this runner.")
        case .refused(let what, _):
            switch what {
            case "prompt", "dialog": return .handoff
            case "draft": return .draftInTerminal
            case "typing": return .said("Someone typed in the terminal in the last 15 seconds, so the message wasn’t sent. Try again once they stop.")
            case "busy": return .said("Claude is working and can’t take a message from here right now.")
            case "too_long": return .said(tooLong)
            case "command": return .said(command)
            case "paste_left": return .said("The message didn’t land in the box as typed, so it was left there and not sent.")
            case "left_at_shell": return .said("Claude quit as the message was typed. It wasn’t run.")
            case "unconfirmed":
                return .said("Claude didn’t confirm it queued the message. Check the terminal before sending it again.")
            case "not_running", "not_an_agent": return .said("Claude isn’t running in this pane.")
            case "unfamiliar", "unproven": return .said("Far Cooler can’t read this terminal’s box, so nothing was typed.")
            default: return .said("The message wasn’t sent.")
            }
        // Never "wasn't sent" for a call that may have arrived: the runner
        // may type it yet, and a second send would go in twice.
        case .timedOut, .lost(notSent: false): return .said(mayHaveBeenSent)
        case .lost(notSent: true): return .said("The runner isn’t connected, so the message wasn’t sent.")
        }
    }

    public static let tooLong = "That message is over \(longest) characters. Shorten it, or paste it in the terminal."
    public static let command =
        "A message can’t start with a symbol Claude reads as a command, such as / or !. Use the terminal for commands."
    public static let mayHaveBeenSent =
        "The runner didn’t answer in time. The message may have been sent, so check the terminal before sending it again."
    public static let draftInTerminal = "The terminal’s box already holds a draft. Send or clear it there first."
    public static let handoff = "Claude is showing something only the terminal can."

    /// Messages claude's queue took that its transcript hasn't shown yet,
    /// once the newest rows show them: as a Queued row, or as the turn each
    /// became.
    public static func unsettled(_ queued: [String], newest rows: [AgentRow]) -> [String] {
        guard !queued.isEmpty else { return queued }
        let shown = Set(rows.compactMap { row -> String? in
            switch row.kind {
            case .queued(let q): q.text
            case .turn(let t): t.prompt
            default: nil
            }
        })
        return queued.filter { !shown.contains($0) }
    }

    // MARK: - Words for rows

    public static func gap(_ gap: AgentRow.Gap) -> String {
        gap.count > 1 ? "Some of this session couldn’t be read." : "A line of this session couldn’t be read."
    }

    /// "0:04", "1:12", "2:03:09": the format a running timer counts in, so
    /// a finished time reads like the running one did.
    public static func short(ms: Int64) -> String {
        let seconds = max(0, ms / 1000)
        let (h, m, s) = (seconds / 3600, (seconds % 3600) / 60, seconds % 60)
        return h > 0 ? String(format: "%d:%02d:%02d", h, m, s) : String(format: "%d:%02d", m, s)
    }

    /// A subagent's type as words: `general-purpose` reads "General purpose".
    public static func agentType(_ raw: String) -> String {
        let words = raw.replacingOccurrences(of: "-", with: " ").replacingOccurrences(of: "_", with: " ")
        guard let first = words.first else { return "Agent" }
        return first.uppercased() + words.dropFirst()
    }

    /// A turn nobody typed: a background task finishing, or claude waking
    /// itself (`TurnOrigin::Notification`, `System`). Drawn as a notice,
    /// never as the person's message.
    public static func isNotice(_ turn: AgentRow.Turn) -> Bool {
        turn.origin == "Notification" || turn.origin == "System"
    }

    /// A notice turn's words, without claude's straight quotes.
    public static func noticeText(_ turn: AgentRow.Turn) -> String {
        turn.prompt.replacingOccurrences(of: "\"", with: "")
    }

    public static func outcome(_ turn: AgentRow.Turn) -> String? {
        switch turn.outcome {
        case nil: nil
        case .finished?: turn.durationMs.map { "Took \(short(ms: $0))" } ?? "Done"
        case .interrupted?: "Interrupted"
        case .unrecorded?: "Not recorded"
        case .failed(let detail)?: detail.isEmpty ? "Failed" : "Failed: \(detail)"
        case .other?: "Ended"
        }
    }

    public static func askTitle(_ ask: AgentRow.Ask) -> String {
        if ask.answered { return "Answered" }
        switch ask.kind {
        case "Permission": return "Claude is asking for permission"
        case "PlanExit": return "Claude has a plan for you to review"
        default: return "Claude is asking a question"
        }
    }

    public static func queuedLabel(_ state: String) -> String {
        switch state {
        case "Withdrawn": "Withdrawn"
        case "Sent": "Sent from the queue"
        default: "Queued"
        }
    }
}
