import Foundation

/// Bounded projector-owned intent. This is never a receipt or navigation authority.
public struct LiteralOperation: Sendable, Equatable, Codable {
    public var version: Int
    public var provenance: String
    public var kind: String
    public var target: String
    public var preview: String?
    public var previewClipped: Bool
    public var lifecycle: Lifecycle
    public var inputClipped: Bool
    public var outputAvailable: Bool
    public var outputClipped: Bool

    public enum Lifecycle: String, Sendable, Codable {
        case running, completed, failed, declined, unconfirmed
        public var words: String {
            switch self {
            case .running: "Command running"
            case .completed: "Command completed"
            case .failed: "Command failed"
            case .declined: "Command declined"
            case .unconfirmed: "Outcome unconfirmed"
            }
        }
    }
    static let noteKinds = ["decision", "finding", "question", "answer", "progress", "comment"]
    static let taskStates = ["backlog", "todo", "needs_decision", "in_progress", "in_review", "done", "cancelled"]
    var valid: Bool {
        let known = kind == "task_show" || kind == "message_orchestrator"
            || Self.noteKinds.contains(where: { kind == "task_note_\($0)" })
            || Self.taskStates.contains(where: { kind == "task_set_\($0)" })
        let taskKey = target.range(of: "^[A-Za-z0-9]+-[0-9]+$", options: .regularExpression) == (target.startIndex..<target.endIndex)
        return version == 1 && provenance == "literal_intent" && known && target.utf8.count <= 64
            && (kind == "message_orchestrator" ? target == "orchestrator" : taskKey)
            && (preview?.unicodeScalars.count ?? 0) <= 280 && (preview?.split(separator: "\n", omittingEmptySubsequences: false).count ?? 0) <= 4
            && ((try? JSONEncoder().encode(self).count) ?? 1_025) <= 1_024
    }
    static func wire(_ object: Any?) -> LiteralOperation? {
        guard let object = object as? [String: Any], JSONSerialization.isValidJSONObject(object),
              let data = try? JSONSerialization.data(withJSONObject: object), data.count <= 1_024 else { return nil }
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        guard let value = try? decoder.decode(Self.self, from: data), value.valid else { return nil }
        return value
    }
    public var verb: String {
        if kind == "task_show" { return "Read task" }
        if kind == "message_orchestrator" { return "Message orchestrator" }
        if kind.hasPrefix("task_note_") {
            let note = String(kind.dropFirst("task_note_".count))
            return note == "decision" ? "Add decision" : "Add \(note) note"
        }
        return "Request status \(kind.dropFirst("task_set_".count).replacingOccurrences(of: "_", with: " "))"
    }
    public var summary: String { kind == "message_orchestrator" ? verb : "\(verb) · \(target)" }
    public var status: AgentRow.Status {
        switch lifecycle {
        case .running: .running
        case .completed: .done
        case .failed, .declined: .failed
        case .unconfirmed: .ended("Outcome unconfirmed")
        }
    }
    public var containsControls: Bool {
        preview?.unicodeScalars.contains(where: { (CharacterSet.controlCharacters.contains($0) && $0 != "\n" && $0 != "\t") || (0x202A...0x202E).contains($0.value) || (0x2066...0x2069).contains($0.value) }) ?? false
    }
}

extension AgentRow.Tool {
    public var presentationName: String { operation == nil ? name : "Far Cooler" }
    public var presentationSummary: String { operation?.summary ?? summary }
    public var presentationStatus: AgentRow.Status { operation?.status ?? status }
    public var presentationAccessibility: String {
        if let operation { return "Far Cooler, recognized command, \(operation.summary), \(operation.lifecycle.words)" }
        let statusWord: String
        switch status {
        case .running: statusWord = "Running"
        case .done: statusWord = "Done"
        case .failed: statusWord = "Failed"
        case .ended: statusWord = "Ended"
        }
        return [name, summary, statusWord].filter { !$0.isEmpty }.joined(separator: ", ")
    }

    /// Cache decoding is independent of wire decoding. Bad optional metadata cannot lose the row.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.init(name: try c.decode(String.self, forKey: .name), summary: try c.decode(String.self, forKey: .summary),
                  status: try c.decode(AgentRow.Status.self, forKey: .status),
                  startedMs: try c.decodeIfPresent(Int64.self, forKey: .startedMs), endedMs: try c.decodeIfPresent(Int64.self, forKey: .endedMs),
                  diff: try c.decode([AgentRow.Hunk].self, forKey: .diff), filePath: try c.decodeIfPresent(String.self, forKey: .filePath),
                  input: try c.decodeIfPresent(String.self, forKey: .input), result: try c.decodeIfPresent(String.self, forKey: .result),
                  operation: nil)
        if let value = try? c.decode(LiteralOperation.self, forKey: .operation), value.valid { operation = value }
    }
}

extension AgentConversation {
    public static func recognizedGroupLabel(_ tools: [AgentRow.Tool]) -> String? {
        guard tools.contains(where: { $0.operation != nil }) else { return nil }
        let words: String
        switch groupStatus(tools) {
        case .running: words = "Commands running"
        case .done: words = "Commands completed"
        case .failed: words = "Command failed or declined"
        case .ended: words = "Outcome unconfirmed"
        }
        return "Recognized commands · \(words)"
    }
}
