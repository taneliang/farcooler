import Foundation

/// The conversation view of a terminal-mode claude or codex pane on a phone
/// (ov-373, ov-416):
/// the rules a phone decides by, here so `swift test` reads them back. The
/// Mac's native view (ov-372) reads the same words.
extension AgentConversation {
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

    /// Whether the composer's field takes the draft from outside: only when
    /// they differ, never mid-composition, and never when the draft is just
    /// something the person typed a moment ago. While the keyboard holds
    /// marked text (a Japanese or Chinese input method's unconfirmed
    /// syllables), the field shows what the draft doesn't hold yet, and
    /// setting the field's text then would throw the composition away, as the
    /// Mac's field already refuses to (ov-470 investigation). A draft that
    /// lags the field by a keystroke or two is an echo of earlier typing
    /// (`DraftEcho`); taking it would delete the characters typed since
    /// (ov-492: "Anthing" after typing "Anything").
    public static func fieldTakesDraft(_ draft: String, showing: String, composing: Bool, echo: DraftEcho = DraftEcho()) -> Bool {
        draft != showing && !composing && !echo.holds(draft)
    }

    /// What the person typed into the field since it last took a draft from
    /// outside. SwiftUI can deliver `updateUIView` with a draft from before
    /// the latest keystroke; a draft in this list is the person's own typing
    /// arriving late, not a restore, a send clearing the box or Bring Here.
    public struct DraftEcho: Equatable, Sendable {
        private var typed: [String] = []
        /// Enough for any lag SwiftUI shows; older entries cannot be echoes.
        static let remembered = 64
        public init() {}

        public func holds(_ draft: String) -> Bool { typed.contains(draft) }

        /// The field's text after a keystroke.
        public mutating func typed(_ text: String) {
            // Never the empty text: a send clears the box to it, and that is
            // from outside however often the person emptied the box before.
            if text.isEmpty || typed.last == text { return }
            typed.append(text)
            if typed.count > Self.remembered { typed.removeFirst(typed.count - Self.remembered) }
        }

        /// The field took a draft from outside: what was typed before it is
        /// no longer an echo of anything.
        public mutating func took(_ draft: String) { typed = [] }
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
        /// Bring Here where it's offered (`BringHere`), and Show Terminal.
        case draftInTerminal
        /// Bring Here put the box's text in the composer, but couldn't clear
        /// the box: the text is in both, as this says. Show Terminal.
        case draftLeftInTerminal(String)
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
    /// `images` means; `agent` names the agent in the words (`conversation_words`).
    public static func issue(for failure: SendFailure, command: Bool = false, agent: String = "Claude") -> SendIssue {
        let words = ConversationWords.table(agent)
        switch failure {
        case .refused(let what, let word):
            let found = word.flatMap { words?.send["word:" + $0] } ?? (command ? words?.send[(what ?? "") + ":command"] : nil)
                ?? words?.send[what ?? ""] ?? words?.send[""]
            return found.flatMap(sendIssue) ?? .said("")
        // Never "wasn't sent" for a call that may have arrived: a second send would go in twice.
        case .timedOut, .lost(notSent: false): return .said(mayHaveBeenSent)
        case .lost(notSent: true): return .said(ConversationWords.phrase("not_connected"))
        }
    }

    static func sendIssue(_ found: ConversationWords.Issue) -> SendIssue? {
        switch found.kind {
        case .handoff: .handoff
        case .panel: .panel
        case .draft: .draftInTerminal
        case .said: .said(found.text ?? "")
        case .quiet: nil
        }
    }

    /// The sentence for refusal word `what`, as `issue` says it.
    public static func refused(_ what: String, _ agent: String = "Claude") -> String {
        ConversationWords.table(agent)?.send[what]?.text ?? ""
    }

    public static var tooLong: String { refused("too_long") }
    public static var tooLongComposed: String { ConversationWords.phrase("too_long_composed") }
    public static var command: String { ConversationWords.phrase("command") }
    public static var tooManyImages: String { refused("images") }
    public static var mayHaveBeenSent: String { ConversationWords.phrase("may_have_been_sent") }
    public static var draftInTerminal: String { ConversationWords.phrase("draft_in_terminal") }
    /// `draftInTerminal`, where Bring Here is offered beside Show Terminal.
    public static var draftInTerminalBring: String { ConversationWords.phrase("draft_in_terminal_bring") }
    public static func panel(_ agent: String) -> String { ConversationWords.phrase("panel", agent: agent) }
    public static func handoff(_ agent: String) -> String { ConversationWords.phrase("handoff", agent: agent) }

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

    // MARK: - claude's suggested prompt (ov-409)

    /// The prompt the composer offers as its placeholder: the one claude's own
    /// box shows after a turn, carried on the newest turn's row. Only while
    /// the rows are live, the agent isn't working or holding a dialog (its
    /// box then shows hints, not predictions), and nothing is typed. A
    /// draft the person may take, never a message: nothing here sends.
    public static func suggestion(newestTurn turn: AgentRow.Turn?, draft: String, stale: Bool) -> String? {
        guard !stale, let turn, turn.activity != "Busy", turn.activity != "Waiting" else { return nil }
        guard draft.isEmpty else { return nil }
        guard let words = turn.suggestion?.trimmingCharacters(in: .whitespacesAndNewlines), !words.isEmpty else { return nil }
        return words
    }

    /// claude's generic `Try "…"` example, shown as the composer's placeholder
    /// in place of "Message Claude" (ov-409): only while the rows are live and
    /// nothing is typed. It is a hint: Tab and a tap do not take it, as in
    /// claude.
    public static func hint(_ text: String?, draft: String, stale: Bool) -> String? {
        guard !stale, draft.isEmpty else { return nil }
        guard let words = text?.trimmingCharacters(in: .whitespacesAndNewlines), !words.isEmpty else { return nil }
        return words
    }

    /// What the composer says when the runner didn't press `key`, or nil when
    /// there's nothing to say: the turn ended on its own (`idle`), or a
    /// second press came too soon after the first (`too_soon`).
    public static func keyIssue(for failure: SendFailure, _ key: PaneKey) -> SendIssue? {
        let words = ConversationWords.table()
        switch failure {
        case .refused(let what, let word):
            let table = key == .stop ? words?.stop : words?.sendNow
            let found = word.flatMap { table?["word:" + $0] } ?? table?[what ?? ""] ?? table?[""]
            return found.flatMap(sendIssue)
        case .timedOut, .lost(notSent: false): return .said(ConversationWords.phrase("key_timed_out"))
        case .lost(notSent: true): return .said(ConversationWords.phrase("key_not_connected"))
        }
    }

    // MARK: - Words for rows

    public static func gap(_ gap: AgentRow.Gap) -> String {
        ConversationWords.phrase(gap.count > 1 ? "gap" : "gap_one")
    }

    /// "0:04", "1:12", "2:03:09": the format a running timer counts in, so
    /// a finished time reads like the running one did.

    /// What a subagent row says of its work: "1 tool", "14 tools", or nothing
    /// while the count is unknown. 0 is how an agent with no transcript read
    /// (its notification may carry no count either) looks, and "0 tools" would
    /// be a claim nobody made (ov-490).
    public static func toolsLabel(_ count: Int) -> String? {
        count <= 0 ? nil : ConversationWords.phrase(count == 1 ? "tools_one" : "tools", ["n": "\(count)"])
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
        let say = { (name: String, fill: [String: String]) in ConversationWords.phrase("outcome_" + name, fill) }
        switch turn.outcome {
        case nil: return nil
        case .finished?: return turn.durationMs.map { say("took", ["time": short(ms: $0)]) } ?? say("done", [:])
        case .interrupted?: return say("interrupted", [:])
        case .unrecorded?: return say("unrecorded", [:])
        case .failed(let detail)?: return detail.isEmpty ? say("failed", [:]) : say("failed_detail", ["detail": detail])
        case .other?: return say("ended", [:])
        }
    }

    /// `viewer` is who this device is on the runner (`RunnerPeople.you`: ""
    /// for the owner, a member's handle, nil if not told). On a runner with
    /// members, an answered ask says who answered (`Ask.answeredSentence`);
    /// a solo runner, and a viewer not yet told, read as they always did.
    public static func askTitle(_ ask: AgentRow.Ask, agent: String = "Claude", viewer: String? = nil) -> String {
        if ask.answered || ask.held == nil, let who = ask.answeredSentence(viewer: viewer) { return who }
        if let by = ask.answeredBy, ask.answered || ask.held == nil { return ConversationWords.phrase("ask_answered_on", ["by": by]) }
        if ask.answered { return ConversationWords.phrase("ask_answered") }
        return ConversationWords.phrase("ask", agent: agent, by: ask.kind)
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
        let answer = ConversationWords.table(agent)?.answer
        return (timedOut ? answer?["timed_out"] : answer?[what ?? ""]) ?? answer?[""] ?? ""
    }

    /// A Queued row's label. `Sent` is a message claude took into the turn
    /// it was running; one sent as a turn of its own is that turn, and its
    /// row is gone (ov-452). `InBox` is a message in claude's box that the
    /// runner sends when the turn ends (ov-827).
    public static func queuedLabel(_ state: String) -> String {
        ConversationWords.phrase("queued", by: state)
    }

/// Where a held ask's answer goes (ov-370): the runner's
/// `terminal.agent_answer`, which writes it to the hook claude is waiting on.
/// The first answer from any device wins; a later one is refused `not_held`.
public protocol AgentAnswerSink: Sendable {
    func answer(terminal: String, ask: String, option: String, answers: [String: String]) async throws
}

extension AgentRowStore {
    /// The newest turn held, whose row carries what claude's box shows now.
    /// Observes `ids` and that turn's row, never the rows after it, which
    /// change on every streamed word (ov-691).
    public var newestTurn: AgentRow.Turn? {
        for id in ids.reversed() {
            guard let box = box(id), box.isTurn else { continue }
            if case .turn(let turn) = box.row.kind { return turn }
        }
        return nil
    }

    /// The ids a transcript draws: every row but the hint (`AgentRow.hintID`),
    /// which is the composer's. Filtered by id, so a view reads no row.
    public var shownIds: [String] { ids.filter { $0 != AgentRow.hintID } }

    /// The `Try` example claude's box shows, for a composer whose draft is
    /// `draft` (`AgentConversation.hint`).
    public func hint(draft: String) -> String? {
        var text: String?
        if case .hint(let hint)? = box(AgentRow.hintID)?.row.kind { text = hint.text }
        return AgentConversation.hint(text, draft: draft, stale: isStale)
    }

    /// The prompt claude suggests, for a composer whose draft is `draft`
    /// (`AgentConversation.suggestion`).
    public func suggestion(draft: String) -> String? {
        AgentConversation.suggestion(newestTurn: newestTurn, draft: draft, stale: isStale)
    }
}

/// An agent conversation's words, from the Rust core's `conversation_words`
/// (ov-1049, R-112) through the plan view's bridge: one table per agent, asked once.
public enum ConversationWords {
    public struct Issue: Decodable, Sendable {
        public enum Kind: String, Decodable, Sendable { case handoff, panel, draft, said, quiet }
        public var kind: Kind
        public var text: String?
    }

    public struct Group: Decodable, Sendable {
        public var names: [String], running: String, done: String
    }

    public struct Table: Decodable, Sendable {
        public var send, stop, sendNow: [String: Issue]
        public var answer, phrases: [String: String]
        public var groups: [Group]

        enum CodingKeys: String, CodingKey {
            case send, stop, answer, phrases, groups
            case sendNow = "send_now"
        }
    }

    private struct Reply: Decodable { var conversation: Table? }

    private static let lock = NSLock()
    nonisolated(unsafe) private static var asked: [String: Table] = [:]

    /// The table for `agent` ("Claude", "Codex"); nil until the bridge answers.
    public static func table(_ agent: String = "Claude") -> Table? {
        if let found = lock.withLock({ asked[agent] }) { return found }
        let request = #"{"schema":1,"conversation":{"style":"apple","agent":"\#(agent)"}}"#
        guard let answer = PlanViewPresenter.ask(request),
              let table = try? JSONDecoder().decode(Reply.self, from: Data(answer.utf8)).conversation
        else { return nil }
        lock.withLock { asked[agent] = table }
        return table
    }

    /// A phrase with its `{placeholders}` filled: `name_<word>` where the table
    /// has one ("queued_InBox"), else `name`; "" while the bridge can't answer.
    public static func phrase(_ name: String, agent: String = "Claude", by word: String? = nil, _ fill: [String: String] = [:]) -> String {
        let phrases = table(agent)?.phrases
        let words = word.flatMap { phrases?[name + "_" + $0] } ?? phrases?[name] ?? ""
        return fill.reduce(words) { $0.replacingOccurrences(of: "{\($1.key)}", with: $1.value) }
    }
}
