import Foundation

// When a card starts, who is working it, and the sentences that say so
// (ov-212, ov-213).
//
// Beside `TaskBoardAgents` and for its reason: the Mac and the iPhone both
// draw these words, and Android transcribes them (`TaskBoard.kt`), so they are
// composed here once and checked against ONE fixture, `task_start_lines.json`,
// that both suites read. The runner sends stable words (`in_line`, `release`,
// `finished`); every sentence a person reads is made here, never there.
//
// Written out, not taken from a formatter, for `TaskRow.listed`'s reason: the
// suite's assertions are English, so ordinals and spans are spelled by hand.
// The two clock times are the exception, and are the user's locale's.

/// Which of the board's two lines a waiting task is standing in.
public enum TaskLine: String, Sendable, Hashable {
    /// Not started: the order tasks get a free agent in.
    case agent
    /// Started: the order tasks get the one build slot in.
    case build
}

/// What a task held for an event is waiting for.
public enum TaskWaitEvent: String, Sendable, Hashable {
    case release
    case recurrence
    case clearBoard = "clear_board"
}

/// What a task is waiting for before it starts. A word this build doesn't know
/// is no wait at all (nil on the row), never a guess.
public enum TaskWait: Equatable, Sendable, Hashable {
    /// `position` 1 is next; 0 is on the line with no rank yet.
    case inLine(TaskLine, position: Int)
    case until(Date)
    case after(TaskWaitEvent)
    case parked
}

/// What the runner knows of one subagent working a task.
public enum TaskWorkerState: String, Sendable, Hashable {
    case running
    /// Open, and the runner can't see it: only what was recorded.
    case unobserved
    case finished
    /// Failed, killed or stopped.
    case stopped

    /// Still open: the orchestrator hasn't closed it out.
    public var isOpen: Bool { self == .running || self == .unobserved }
}

/// One subagent working a task from inside another agent's session.
public struct TaskWorker: Equatable, Sendable, Hashable {
    public var harness: String
    public var state: TaskWorkerState
    public var startedAt: Date?
    public var endedAt: Date?
    /// What it's doing right now, in the runner's words, or empty.
    public var doing: String
    /// The pane its orchestrator runs in: where the subagent lives.
    public var orchestratorTerminalID: String?

    public init(
        harness: String, state: TaskWorkerState, startedAt: Date? = nil, endedAt: Date? = nil,
        doing: String = "", orchestratorTerminalID: String? = nil
    ) {
        self.harness = harness
        self.state = state
        self.startedAt = startedAt
        self.endedAt = endedAt
        self.doing = doing
        self.orchestratorTerminalID = orchestratorTerminalID
    }
}

/// How the subagent control reads: some are still at it, or the last one is
/// done and the card hasn't moved.
public enum SubagentState: Equatable, Sendable, Hashable {
    case working
    case finished
    case stopped
}

extension TaskRow {
    /// The subagents that are still open.
    public var openWorkers: [TaskWorker] { workers.filter { $0.state.isOpen } }

    /// The pane the subagents live in, for the control to open: the first
    /// open worker's orchestrator, else the last one's.
    public var orchestratorTerminalID: String? {
        (openWorkers + workers.reversed()).compactMap(\.orchestratorTerminalID).first
    }

    /// What the Agent control says about subagents, or nil: only a started
    /// card has them, and only when the runner recorded some.
    ///
    /// Open ones count; with none open the one most recently closed stands
    /// for the work, because a card still In Progress after its subagent
    /// finished is the orchestrator reviewing or landing it.
    var subagentPresence: TaskAgentPresence? {
        guard status == .inProgress, !workers.isEmpty else { return nil }
        let open = openWorkers
        if !open.isEmpty { return .subagents(open.count, .working) }
        let last = workers.max { ($0.endedAt ?? .distantPast) < ($1.endedAt ?? .distantPast) }
        return .subagents(1, last?.state == .stopped ? .stopped : .finished)
    }

    /// The card's quiet second line about when it starts and who's on it, or
    /// nil. Sentence case; body text, never a control.
    ///
    /// Two parts: why it hasn't started or is waiting (the runner's wait,
    /// shown only in the statuses where it means something, so an older
    /// runner that moved a task without clearing its wait can't put a stale
    /// line on screen), then the subagent. A running subagent is the
    /// explanation, so it speaks only when there's no wait; a finished one
    /// speaks beside the wait ("Waiting to build, 2nd in line · Subagent
    /// finished 12 min ago").
    ///
    /// `speaksOfAgents` is `TaskAgentLink.speaksOfAgents`: false when the
    /// runner can't be believed about its agents right now, which drops the
    /// subagent part and leaves the board's own wait.
    public func startLine(
        at now: Date, speaksOfAgents: Bool = true, timeZone: TimeZone = .current,
        locale: Locale = .current
    ) -> String? {
        let wait = waitSentence(at: now, timeZone: timeZone, locale: locale)
        var parts = [String]()
        if let wait { parts.append(wait) }
        if speaksOfAgents, status == .inProgress, !workers.isEmpty {
            let running = !openWorkers.isEmpty
            if !(running && wait != nil), let worker = workerSentence(at: now) {
                parts.append(worker)
            }
        }
        if parts.isEmpty, status == .todo, blockedBy.isEmpty { return "Ready to start" }
        return parts.isEmpty ? nil : parts.joined(separator: " · ")
    }

    private func waitSentence(at now: Date, timeZone: TimeZone, locale: Locale) -> String? {
        guard let wait else { return nil }
        switch wait {
        case .inLine(.agent, let position):
            guard status == .backlog || status == .todo, position > 0 else { return nil }
            return position == 1 ? "Next to start" : "\(TaskRow.ordinal(position)) in line"
        case .inLine(.build, let position):
            guard status == .inProgress || status == .inReview, position > 0 else { return nil }
            return position == 1
                ? "Builds next" : "Waiting to build, \(TaskRow.ordinal(position)) in line"
        case .until(let date):
            // A time that has come reads as an ordinary Backlog card: the
            // runner clears it a moment later, and "Starts at 9:00 AM" in the
            // past is the one wrong thing to say meanwhile.
            guard status == .backlog, date > now else { return nil }
            return "Starts " + TaskRow.when(date, now: now, timeZone: timeZone, locale: locale)
        case .after(let event):
            guard status == .backlog else { return nil }
            switch event {
            case .release: return "Starts after the next release"
            case .recurrence: return "Starts if it happens again"
            case .clearBoard: return "Starts when nothing else is waiting"
            }
        case .parked:
            return status == .backlog ? "Not planned" : nil
        }
    }

    private func workerSentence(at now: Date) -> String? {
        let open = openWorkers
        if open.count > 1 { return "\(open.count) subagents working" }
        if let worker = open.first {
            let who = TaskRow.harnessWord(worker.harness)
            let observed = worker.state == .running && who != "Codex"
            guard observed else {
                let head = who.isEmpty ? "Subagent" : "\(who) subagent"
                guard let started = worker.startedAt else { return head }
                return "\(head), started \(TaskRow.ago(from: started, to: now))"
            }
            var line = who.isEmpty ? "Subagent working" : "\(who) subagent working"
            if let started = worker.startedAt { line += ", \(TaskRow.span(from: started, to: now))" }
            if !worker.doing.isEmpty { line += " · \(worker.doing)" }
            return line
        }
        guard let last = workers.max(by: { ($0.endedAt ?? .distantPast) < ($1.endedAt ?? .distantPast) })
        else { return nil }
        let verb = last.state == .stopped ? "stopped" : "finished"
        guard let ended = last.endedAt else { return "Subagent \(verb)" }
        return "Subagent \(verb) \(TaskRow.ago(from: ended, to: now))"
    }

    /// The runner's `claude` and `codex`, as a person spells them; anything
    /// else is no name at all.
    static func harnessWord(_ harness: String) -> String {
        switch harness {
        case "claude": return "Claude"
        case "codex": return "Codex"
        default: return ""
        }
    }

    /// `1st`, `2nd`, `3rd`, `4th`, `11th`, `12th`, `21st`.
    static func ordinal(_ n: Int) -> String {
        let tens = n % 100
        if (11...13).contains(tens) { return "\(n)th" }
        switch n % 10 {
        case 1: return "\(n)st"
        case 2: return "\(n)nd"
        case 3: return "\(n)rd"
        default: return "\(n)th"
        }
    }

    /// `12 min ago`, `3 h ago`, `2 d ago`, or `just now` under a minute.
    static func ago(from then: Date, to now: Date) -> String {
        let minutes = max(0, Int(now.timeIntervalSince(then))) / 60
        if minutes < 1 { return "just now" }
        return span(minutes: minutes) + " ago"
    }

    /// `12 min`, `3 h`, `2 d`, or `under 1 min`.
    static func span(from then: Date, to now: Date) -> String {
        let minutes = max(0, Int(now.timeIntervalSince(then))) / 60
        return minutes < 1 ? "under 1 min" : span(minutes: minutes)
    }

    private static func span(minutes: Int) -> String {
        if minutes < 60 { return "\(minutes) min" }
        if minutes < 24 * 60 { return "\(minutes / 60) h" }
        return "\(minutes / (24 * 60)) d"
    }

    /// `at 9:00 AM`, `tomorrow at 9:00 AM`, or `Mon, Oct 5 at 9:00 AM`, in the
    /// given zone and locale. The narrow no-break space newer ICU puts before
    /// AM is a plain space, so the same words come out wherever they're made.
    static func when(_ date: Date, now: Date, timeZone: TimeZone, locale: Locale) -> String {
        var calendar = Calendar(identifier: .gregorian)
        calendar.timeZone = timeZone
        calendar.locale = locale
        let time = DateFormatter()
        time.locale = locale
        time.timeZone = timeZone
        time.dateStyle = .none
        time.timeStyle = .short
        let clock = time.string(from: date).replacingOccurrences(of: "\u{202F}", with: " ")
        let days =
            calendar.dateComponents(
                [.day], from: calendar.startOfDay(for: now), to: calendar.startOfDay(for: date)
            ).day ?? 0
        switch days {
        case ..<1: return "at \(clock)"
        case 1: return "tomorrow at \(clock)"
        default:
            let day = DateFormatter()
            day.locale = locale
            day.timeZone = timeZone
            day.setLocalizedDateFormatFromTemplate("EEEMMMd")
            return "\(day.string(from: date)) at \(clock)"
        }
    }
}

// ---------------------------------------------------------------------------
// The wire
//
// `"wait"`, `"waiting_on"` and `"workers"` on a row, as `task_starts_json`
// writes them. Decoded leniently, by `WireTask`'s rule: a row from an older
// runner has none of them, and one a newer runner words differently loses the
// piece it can't read, never the board.
// ---------------------------------------------------------------------------

/// A row's `"wait"` object.
struct WireWait: Decodable {
    var kind: String
    var line: String?
    var position: Int?
    var until: Int64?
    var event: String?

    var wait: TaskWait? {
        switch kind {
        case "in_line":
            guard let line = TaskLine(rawValue: line ?? "") else { return nil }
            return .inLine(line, position: position ?? 0)
        case "until":
            guard let until, until > 0 else { return nil }
            return .until(Date(timeIntervalSince1970: Double(until) / 1000))
        case "after":
            return TaskWaitEvent(rawValue: event ?? "").map(TaskWait.after)
        case "parked": return .parked
        default: return nil
        }
    }
}

/// One entry of a row's `"workers"`.
struct WireWorker: Decodable {
    var harness: String
    var state: String
    var startedAt: Int64?
    var endedAt: Int64?
    var doing: String?
    var orchestratorTerminal: String?

    enum CodingKeys: String, CodingKey {
        case harness, state, doing
        case startedAt = "started_at"
        case endedAt = "ended_at"
        case orchestratorTerminal = "orchestrator_terminal"
    }

    private static func date(_ ms: Int64?) -> Date? {
        ms.flatMap { $0 > 0 ? Date(timeIntervalSince1970: Double($0) / 1000) : nil }
    }

    /// An unknown state is `unobserved`: open, and nothing more is claimed.
    var worker: TaskWorker {
        TaskWorker(
            harness: harness, state: TaskWorkerState(rawValue: state) ?? .unobserved,
            startedAt: Self.date(startedAt), endedAt: Self.date(endedAt), doing: doing ?? "",
            orchestratorTerminalID: orchestratorTerminal)
    }
}
