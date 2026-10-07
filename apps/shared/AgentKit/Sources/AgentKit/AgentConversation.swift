import Foundation

/// The conversation view of a terminal-mode claude or codex pane on a phone
/// (ov-373, ov-416):
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

    /// Whether a pane is an agent in a terminal the runner projects rows
    /// for and composes into: claude, or codex where the runner says it does
    /// (`codex_view`, ov-416). A runner from before projected no codex rows
    /// and refused every send into one.
    public static func isAgentInATerminal(paneMode: String?, preset: String, build: DaemonBuild?) -> Bool {
        guard (paneMode ?? "terminal") == "terminal" else { return false }
        if preset.hasPrefix("claude") { return true }
        return preset.hasPrefix("codex") && build?.can(.codexView) == true
    }

    /// The agent's name, as the conversation's words say it.
    public static func agentName(preset: String) -> String {
        preset.hasPrefix("codex") ? "Codex" : "Claude"
    }

    /// Whether the runner presses Stop and Send Now in this agent's pane:
    /// claude's keys alone (`terminal.interrupt` refuses codex).
    public static func pressesKeys(preset: String) -> Bool {
        preset.hasPrefix("claude")
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

    /// The longest with `compose` (`compose.rs`'s `LONGEST_TEXT`).
    public static let longestComposed = 100_000

    /// The most images in one message (`compose.rs`'s `MOST_IMAGES`).
    public static let mostImages = 10

    /// The longest message the box takes, where the runner has `compose`
    /// (`rich`) or doesn't.
    public static func longest(rich: Bool) -> Int { rich ? longestComposed : longest }

    /// The words that say a message is over `longest(rich:)`.
    public static func tooLong(rich: Bool) -> String { rich ? tooLongComposed : tooLong }

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
        /// The message is one of claude's own commands that opens a panel or
        /// acts at once (`handoff`): the Handoff row, with Show Terminal.
        case panel
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

    /// `command` when the message was a slash command, which says which limit
    /// `images` means; `agent` names the agent in the words.
    public static func issue(for failure: SendFailure, command: Bool = false, agent: String = "Claude") -> SendIssue {
        switch failure {
        // A grant that may read but not type (`read`): saying so beats
        // "wasn't sent" on every try.
        case .refused(_, let word?) where word == "scope-denied":
            return .said("This device can’t send messages to this runner.")
        case .refused(let what, _):
            switch what {
            case "prompt", "dialog": return .handoff
            case "handoff": return .panel
            case "draft": return .draftInTerminal
            case "typing": return .said("Someone typed in the terminal in the last 15 seconds, so the message wasn’t sent. Try again once they stop.")
            case "busy": return .said("\(agent) is working and can’t take a message from here right now.")
            case "too_long": return .said(tooLong)
            case "command": return .said(commandRefused(agent))
            case "paste_left": return .said("The message didn’t land in the box as typed, so it was left there and not sent.")
            case "left_at_shell": return .said("\(agent) quit as the message was typed. It wasn’t run.")
            case "unconfirmed": return .said(unconfirmed(agent))
            case "not_running", "not_an_agent": return .said("\(agent) isn’t running in this pane.")
            case "unfamiliar", "unproven": return .said("Far Cooler can’t read this terminal’s box, so nothing was typed.")
            case "unconfirmable": return .said("Far Cooler can’t find \(agent)’s session to confirm a send, so nothing was typed.")
            case "unsupported": return .said("\(agent) can’t take a message from here. Use the terminal.")
            case "picker": return .said(picker(agent))
            case "too_tall": return .said(tooTall(agent))
            case "images_too_large": return .said(imagesTooLarge)
            case "images": return .said(command ? commandWithImages : tooManyImages)
            case "image_too_large": return .said(imageTooLarge)
            case "backslash": return .said(backslash)
            case "image": return .said("One of the images couldn’t be read, so nothing was sent.")
            default: return .said("The message wasn’t sent.")
            }
        // Never "wasn't sent" for a call that may have arrived: the runner
        // may type it yet, and a second send would go in twice.
        case .timedOut, .lost(notSent: false): return .said(mayHaveBeenSent)
        case .lost(notSent: true): return .said("The runner isn’t connected, so the message wasn’t sent.")
        }
    }

    public static let tooLong = "That message is over \(longest) characters. Shorten it, or paste it in the terminal."
    public static let tooLongComposed =
        "That message is over \(longestComposed.formatted(.number.locale(Locale(identifier: "en_US")))) characters. Shorten it, or paste it in the terminal."
    public static let command =
        "A message can’t start with a symbol Claude reads as a command, such as / or !. Use the terminal for commands."
    /// The runner's `command`: a `!`, which claude's box runs in a shell, or
    /// a `/` before something that isn't a command's name.
    public static let commandRefused = commandRefused("Claude")
    public static func commandRefused(_ agent: String) -> String {
        "\(agent) would run that as a shell command or doesn’t have that command, so it wasn’t sent. Use the terminal for it."
    }
    public static let imagesTooLarge = "These images are too large to send together. Send fewer or smaller ones."
    public static let tooManyImages = "A message takes at most \(mostImages) images."
    public static let commandWithImages = "A slash command can’t carry images. Send it without them."
    public static let imageTooLarge = "That image is too large to send. Use a smaller one."
    public static let backslash =
        "Claude reads a backslash at the end as a new line, so the message wasn’t sent. Remove it, or add a word after it."
    public static let unconfirmed = unconfirmed("Claude")
    public static func unconfirmed(_ agent: String) -> String {
        "\(agent) didn’t confirm it took the message. Check the terminal before sending it again."
    }
    public static let panel = panel("Claude")
    public static func panel(_ agent: String) -> String { "This opens a panel in \(agent), so it’s for the terminal." }
    /// The runner's `picker` (codex, ov-416): a last word codex would open a
    /// picker for, which takes the Enter.
    public static func picker(_ agent: String) -> String {
        "\(agent) would open a picker for a last word that starts with @ or $, so the message wasn’t sent. Add a word after it, or use the terminal."
    }
    /// The runner's `too_tall` (codex, ov-416): more lines than its box
    /// shows, so it couldn't be read back.
    public static func tooTall(_ agent: String) -> String {
        "That message is too tall for \(agent)’s box to show whole, so it wasn’t sent. Shorten it, or paste it in the terminal."
    }
    public static let mayHaveBeenSent =
        "The runner didn’t answer in time. The message may have been sent, so check the terminal before sending it again."
    public static let draftInTerminal = "The terminal’s box already holds a draft. Send or clear it there first."
    public static let handoff = handoff("Claude")
    public static func handoff(_ agent: String) -> String { "\(agent) is showing something only the terminal can." }

    /// Messages claude's queue took that its transcript hasn't shown yet,
    /// once the newest rows show them: as a Queued row, or as the turn each
    /// became. Compared by `words`, so a message with images matches its row
    /// (`[Image #1]` there, `[Image]` here).
    public static func unsettled(_ queued: [String], newest rows: [AgentRow]) -> [String] {
        guard !queued.isEmpty else { return queued }
        let shown = Set(rows.compactMap { row -> String? in
            switch row.kind {
            case .queued(let q): words(q.text)
            case .turn(let t): words(t.prompt)
            default: nil
            }
        })
        return queued.filter { !shown.contains(words($0)) }
    }

    /// A Queued row's words until the transcript shows the message: each
    /// image as `[Image]`, then the text.
    public static func echo(_ text: String, images: Int) -> String {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        return (Array(repeating: "[Image]", count: images) + (trimmed.isEmpty ? [] : [trimmed])).joined(separator: " ")
    }

    /// What an echo and the transcript's row for it share: how many images
    /// the message has (claude's `[Image #N]`, the echo's `[Image]`), and its
    /// words without them or the white space around them. So an echo of two
    /// images is never taken for a row of one.
    public static func words(_ text: String) -> String {
        let placeholder = #"\[Image( #\d+)?\]"#
        let images = (try? Regex(placeholder)).map { text.ranges(of: $0).count } ?? 0
        let words = text.replacingOccurrences(of: placeholder, with: "", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return "\(images) \(words)"
    }

    // MARK: - Stop and Send Now (ov-368)

    /// Which key the runner is asked to press.
    public enum PaneKey: Equatable, Sendable {
        case stop, sendNow
    }

    /// Whether claude is working on a turn, as the newest turn's row says
    /// (claude's registry, `Busy`). Not while a dialog is up: the row says
    /// `Waiting` then, and an Esc would answer the dialog No.
    public static func isWorking(newestTurn turn: AgentRow.Turn?) -> Bool {
        guard let turn else { return false }
        return turn.outcome == nil && turn.activity == "Busy"
    }

    /// The newest turn among `rows`, which may be in any order.
    public static func newestTurn(in rows: [AgentRow]) -> AgentRow.Turn? {
        for row in rows.reversed() {
            if case .turn(let turn) = row.kind { return turn }
        }
        return nil
    }

    /// What the composer says when the runner didn't press `key`, or nil when
    /// there's nothing to say: the turn ended on its own (`idle`), or a
    /// second press came too soon after the first (`too_soon`).
    public static func keyIssue(for failure: SendFailure, _ key: PaneKey) -> SendIssue? {
        let stop = key == .stop
        switch failure {
        case .refused(_, let word?) where word == "scope-denied":
            return .said("This device can’t control this runner.")
        case .refused(let what, _):
            switch what {
            case "idle", "too_soon": return nil
            case "prompt": return .handoff
            case "draft": return .draftInTerminal
            case "typing": return .said("Someone is typing in the terminal. Try again in a moment.")
            case "sending": return .said("A message is still going in. Try again in a moment.")
            case "nothing_queued": return .said("Nothing is waiting in Claude’s queue.")
            case "settling":
                return .said(stop ? "Claude is starting a step. Try Stop again in a moment."
                    : "Claude is starting a step. Try Send Now again in a moment.")
            case "unconfirmed":
                return .said(stop ? "Claude didn’t confirm it stopped. It may have stopped; check the terminal before pressing again."
                    : "Claude didn’t confirm it sent the queued messages. Check the terminal.")
            default:
                return .said(stop ? "Far Cooler can’t stop Claude safely from here. Use the terminal."
                    : "Far Cooler can’t send the queue safely from here. Use the terminal.")
            }
        case .timedOut, .lost(notSent: false):
            return .said("The runner didn’t answer in time. Check the terminal.")
        case .lost(notSent: true):
            return .said("The runner isn’t connected. Use the terminal.")
        }
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

    public static func askTitle(_ ask: AgentRow.Ask, agent: String = "Claude") -> String {
        if let by = ask.answeredBy, ask.answered || ask.held == nil { return "Answered on \(by)" }
        if ask.answered { return "Answered" }
        switch ask.kind {
        case "Permission": return "\(agent) is asking for permission"
        case "PlanExit": return "\(agent) has a plan for you to review"
        default: return "\(agent) is asking a question"
        }
    }

    // MARK: - Answering a held ask (ov-370, R-33)

    /// Whether this view can answer the ask: the runner's hook holds it and
    /// nothing has answered it yet. Otherwise only the terminal can.
    public static func answerable(_ ask: AgentRow.Ask) -> Bool {
        !ask.answered && ask.held != nil
    }

    /// What `terminal.agent_answer` takes for each button. A question is
    /// answered with `answer` and its answers; a plan with `allow` (Approve
    /// Plan) or `deny` (Keep Planning); a permission with `allow` or `deny`.
    public enum AnswerOption {
        public static let allow = "allow"
        public static let deny = "deny"
        public static let answer = "answer"
    }

    /// A question's answers as claude reads them, each question's words to
    /// its answer: the options picked, in the order offered, then any words
    /// typed in Other, joined by ", ". Nil until every question has one.
    public static func answers(
        for questions: [AgentRow.Ask.Question], picked: [Int: Set<String>], typed: [Int: String]
    ) -> [String: String]? {
        guard !questions.isEmpty else { return nil }
        var answers: [String: String] = [:]
        for (i, question) in questions.enumerated() {
            let chosen = question.options.map(\.label).filter { picked[i]?.contains($0) == true }
            let other = (typed[i] ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            // Other is one more choice: for a single-choice question it
            // replaces the pick, as claude's own dialog has it (review 1 L1).
            let parts = other.isEmpty ? chosen : question.multiSelect ? chosen + [other] : [other]
            guard !parts.isEmpty else { return nil }
            answers[question.question] = parts.joined(separator: ", ")
        }
        return answers
    }

    /// One pick: a single-choice question's replaces what was picked, a
    /// multi-select's toggles.
    public static func pick(_ label: String, in question: AgentRow.Ask.Question, picked: Set<String>) -> Set<String> {
        guard question.multiSelect else { return [label] }
        return picked.contains(label) ? picked.subtracting([label]) : picked.union([label])
    }

    /// Why an answer didn't land, by the runner's word for it.
    public static func answerIssue(what: String?, timedOut: Bool = false, agent: String = "Claude") -> String {
        if timedOut { return "The runner didn’t answer in time. Check the terminal before answering again." }
        switch what {
        case "not_held": return "This isn’t waiting here anymore. It was answered, or only the terminal can answer it now."
        // Its hold is over, so a second try would be refused: the terminal.
        case "not_delivered": return "The answer didn’t reach \(agent). Answer in the terminal."
        case "answers": return "Answer every question first."
        default: return "The answer wasn’t sent. Answer in the terminal."
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

/// Where a held ask's answer goes (ov-370): the runner's
/// `terminal.agent_answer`, which writes it to the hook claude is waiting on.
/// The first answer from any device wins; a later one is refused `not_held`.
public protocol AgentAnswerSink: Sendable {
    func answer(terminal: String, ask: String, option: String, answers: [String: String]) async throws
}
