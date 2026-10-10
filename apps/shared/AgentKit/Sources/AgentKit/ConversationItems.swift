import Foundation

/// What a conversation view draws, item by item (ov-452): a row, or a run of
/// tool calls folded into one line that opens to them, as claude's own
/// "Ran 3 shell commands" does.
public enum ConversationItem: Hashable, Identifiable, Sendable {
    case row(String)
    /// Two or more tool calls in a row, with the thinking between them: the
    /// ids of every row it holds, oldest first.
    case tools(id: String, rows: [String])

    public var id: String {
        switch self {
        case .row(let id): id
        case .tools(let id, _): id
        }
    }
}

extension AgentConversation {
    /// What a row is to the grouping: a tool call, thinking (which a group
    /// takes in only between two calls), or anything else, which ends one.
    public enum GroupRole: Sendable, Equatable {
        case tool, thinking, other
    }

    public static func groupRole(_ kind: AgentRow.Kind) -> GroupRole {
        switch kind {
        case .tool: .tool
        case .thinking: .thinking
        default: .other
        }
    }

    /// `ids` as items: each run of two or more tool calls, with the thinking
    /// rows between them, is one `.tools` item; everything else, a tool call
    /// on its own included, is a `.row`. Text, a message, a subagent, a
    /// question or a task list ends a run. Thinking before a run's first call
    /// or after its last stays a row of its own.
    public static func items(_ ids: [String], role: (String) -> GroupRole) -> [ConversationItem] {
        var items: [ConversationItem] = []
        // The open run: rows since its first call, and how many are calls.
        var run: [String] = []
        var calls = 0
        // Thinking seen since the run's last call, not yet known to be inside.
        var trailing: [String] = []

        func close() {
            if calls >= 2, let first = run.first {
                items.append(.tools(id: "tools:\(first)", rows: run))
            } else {
                items.append(contentsOf: run.map(ConversationItem.row))
            }
            items.append(contentsOf: trailing.map(ConversationItem.row))
            run = []
            trailing = []
            calls = 0
        }

        for id in ids {
            switch role(id) {
            case .tool:
                run.append(contentsOf: trailing)
                trailing = []
                run.append(id)
                calls += 1
            case .thinking:
                if run.isEmpty { items.append(.row(id)) } else { trailing.append(id) }
            case .other:
                close()
                items.append(.row(id))
            }
        }
        close()
        return items
    }

    /// A group's line: what its calls did, and how many. "Ran 3 commands"
    /// when every call is a shell command, "Read 2 files", "Edited 4 files",
    /// "Ran 2 searches", else "Used 5 tools"; in the present tense while any
    /// is still running.
    public static func groupTitle(_ tools: [AgentRow.Tool]) -> String {
        let n = tools.count
        let running = tools.contains { $0.status == .running }
        let names = Set(tools.map(\.name))
        func all(_ these: Set<String>) -> Bool { names.isSubset(of: these) }
        if all(["Bash", "bash"]) { return running ? "Running \(n) commands" : "Ran \(n) commands" }
        if all(["Read"]) { return running ? "Reading \(n) files" : "Read \(n) files" }
        if all(["Edit", "MultiEdit", "Write", "NotebookEdit"]) { return running ? "Editing \(n) files" : "Edited \(n) files" }
        if all(["Grep", "Glob"]) { return running ? "Running \(n) searches" : "Ran \(n) searches" }
        return running ? "Using \(n) tools" : "Used \(n) tools"
    }

    /// What a group's calls were for, from their own summaries: a command's
    /// description, a file's name. Each said once, in order, joined by
    /// commas; a view cuts it to fit.
    public static func groupPurpose(_ tools: [AgentRow.Tool]) -> String {
        var seen = Set<String>()
        var phrases: [String] = []
        for tool in tools {
            let summary = tool.summary.trimmingCharacters(in: .whitespacesAndNewlines)
            let phrase = summary.isEmpty ? tool.name : purpose(of: summary)
            if seen.insert(phrase).inserted { phrases.append(phrase) }
        }
        return phrases.joined(separator: ", ")
    }

    /// One summary as a phrase inside a sentence: a path as its file's name,
    /// and a sentence's capital lowered ("Map the code" reads "map the
    /// code"), unless the word is all capitals or a name like "README".
    static func purpose(of summary: String) -> String {
        if summary.hasPrefix("/") || summary.hasPrefix("~/"), !summary.contains(" ") {
            return (summary as NSString).lastPathComponent
        }
        let chars = Array(summary)
        guard chars.count > 1, chars[0].isUppercase, chars[1].isLowercase else { return summary }
        return chars[0].lowercased() + String(chars.dropFirst())
    }

    /// A group's state: running while any call is, failed if any failed,
    /// else done.
    public static func groupStatus(_ tools: [AgentRow.Tool]) -> AgentRow.Status {
        if tools.contains(where: { $0.status == .running }) { return .running }
        if tools.contains(where: { $0.status == .failed }) { return .failed }
        return .done
    }

    // MARK: - Long messages

    /// The lines a long message shows before Show More.
    public static let collapsedLines = 6

    /// Whether `text` runs past `lines` lines at about `width` characters a
    /// line: it then shows that many, with Show More. An estimate on purpose,
    /// so no row is measured to decide.
    public static func isLong(_ text: String, lines: Int = collapsedLines, width: Int = 90) -> Bool {
        var count = 0
        for line in text.split(separator: "\n", omittingEmptySubsequences: false) {
            count += max(1, (line.count + width - 1) / width)
            if count > lines { return true }
        }
        return false
    }

    // MARK: - Where a message came from

    /// What a turn's footer says about where its prompt came from, if
    /// anything: a message typed while the agent worked, sent when its turn
    /// ended.
    public static func originNote(_ turn: AgentRow.Turn) -> String? {
        turn.origin == "Queued" ? "Queued during the last turn" : nil
    }

    /// A scheduled task's firing (`CronCreate`, `/loop`): drawn as one, with
    /// its prompt folded, never as the person's message.
    public static func isScheduled(_ turn: AgentRow.Turn) -> Bool {
        turn.origin == "Scheduled"
    }

    public static let scheduledTask = "Scheduled task"

    // MARK: - Task lists

    /// "2 of 5 done".
    public static func taskProgress(_ tasks: AgentRow.Tasks) -> String {
        let done = tasks.items.filter { $0.status == "Completed" }.count
        return "\(done) of \(tasks.items.count) done"
    }

    /// A task's state, as VoiceOver says it.
    public static func taskState(_ item: AgentRow.TaskItem) -> String {
        switch item.status {
        case "Completed": "Done"
        case "InProgress": "In progress"
        default: "Not started"
        }
    }
}
