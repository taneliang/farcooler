import Foundation

public enum Role: String, Decodable, Sendable {
    case user = "User"
    case agent = "Agent"
    case thought = "Thought"
}

public enum ToolStatus: String, Decodable, Sendable {
    case pending = "Pending"
    case inProgress = "InProgress"
    case completed = "Completed"
    case failed = "Failed"
}

public enum GapReason: String, Decodable, Sendable {
    case ringTrimmed = "RingTrimmed"
    case loadUnsupported = "LoadUnsupported"
    case unparsed = "Unparsed"
}

public struct Diff: Decodable, Sendable, Equatable {
    public let path: String
    public let oldText: String?
    public let newText: String

    public init(path: String, oldText: String?, newText: String) {
        self.path = path
        self.oldText = oldText
        self.newText = newText
    }

    enum CodingKeys: String, CodingKey {
        case path
        case oldText = "old_text"
        case newText = "new_text"
    }
}

public struct PlanEntry: Decodable, Sendable, Equatable {
    public let content: String
    public let priority: String
    public let status: String

    public init(content: String, priority: String, status: String) {
        self.content = content
        self.priority = priority
        self.status = status
    }
}

public struct PermissionOption: Decodable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let kind: String

    public init(id: String, name: String, kind: String) {
        self.id = id
        self.name = name
        self.kind = kind
    }
}

public enum AgentEvent: Sendable, Equatable {
    case sessionStarted(sessionID: String, agentMode: String?, availableModes: [String], availableCommands: [String])
    case message(role: Role, text: String)
    case toolCall(id: String, title: String, kind: String, status: ToolStatus, locations: [String])
    case toolUpdate(id: String, status: ToolStatus, content: String?, diff: Diff?)
    case plan(entries: [PlanEntry])
    case permission(id: String, toolCall: String, options: [PermissionOption])
    case resolved(id: String, chosen: String)
    case modeSet(agentMode: String)
    case turnEnded(reason: String)
    case gap(GapReason)
}

public struct Sequenced: Sendable, Equatable {
    public let seq: UInt64
    public let event: AgentEvent

    public init(seq: UInt64, event: AgentEvent) {
        self.seq = seq
        self.event = event
    }
}

extension AgentEvent {
    /// Decode one serialised `overnight_agent::event::AgentEvent`.
    ///
    /// Serde's externally-tagged representation: a single-key object whose key
    /// names the variant. An unrecognised key is a `.gap(.unparsed)` rather
    /// than a throw, because a client one release behind its daemon must still
    /// render the session — and rather than a silent skip, because a shorter
    /// transcript that looks complete is the one failure this design refuses.
    public static func decode(from json: String) throws -> AgentEvent {
        guard let data = json.data(using: .utf8) else { return .gap(.unparsed) }
        return try decode(from: data)
    }

    public static func decode(from data: Data) throws -> AgentEvent {
        let container = try JSONDecoder().decode(Envelope.self, from: data)
        return container.event
    }

    private struct Envelope: Decodable {
        let event: AgentEvent

        private struct Key: CodingKey {
            var stringValue: String
            var intValue: Int? { nil }
            init?(stringValue: String) { self.stringValue = stringValue }
            init?(intValue: Int) { nil }
        }

        init(from decoder: Decoder) throws {
            let outer = try decoder.container(keyedBy: Key.self)
            guard let key = outer.allKeys.first else {
                self.event = .gap(.unparsed)
                return
            }
            switch key.stringValue {
            case "SessionStarted":
                let p = try outer.decode(SessionStartedPayload.self, forKey: key)
                event = .sessionStarted(
                    sessionID: p.sessionID, agentMode: p.agentMode,
                    availableModes: p.availableModes, availableCommands: p.availableCommands)
            case "Message":
                let p = try outer.decode(MessagePayload.self, forKey: key)
                event = .message(role: p.role, text: p.text)
            case "ToolCall":
                let p = try outer.decode(ToolCallPayload.self, forKey: key)
                event = .toolCall(
                    id: p.id, title: p.title, kind: p.kind, status: p.status,
                    locations: p.locations)
            case "ToolUpdate":
                let p = try outer.decode(ToolUpdatePayload.self, forKey: key)
                event = .toolUpdate(id: p.id, status: p.status, content: p.content, diff: p.diff)
            case "Plan":
                let p = try outer.decode(PlanPayload.self, forKey: key)
                event = .plan(entries: p.entries)
            case "Permission":
                let p = try outer.decode(PermissionPayload.self, forKey: key)
                event = .permission(id: p.id, toolCall: p.toolCall, options: p.options)
            case "Resolved":
                let p = try outer.decode(ResolvedPayload.self, forKey: key)
                event = .resolved(id: p.id, chosen: p.chosen)
            case "ModeSet":
                let p = try outer.decode(ModeSetPayload.self, forKey: key)
                event = .modeSet(agentMode: p.agentMode)
            case "TurnEnded":
                let p = try outer.decode(TurnEndedPayload.self, forKey: key)
                event = .turnEnded(reason: p.reason)
            case "Gap":
                let p = try outer.decode(GapPayload.self, forKey: key)
                event = .gap(p.reason)
            default:
                event = .gap(.unparsed)
            }
        }
    }

    private struct SessionStartedPayload: Decodable {
        let sessionID: String
        let agentMode: String?
        let availableModes: [String]
        let availableCommands: [String]
        enum CodingKeys: String, CodingKey {
            case sessionID = "session_id"
            case agentMode = "agent_mode"
            case availableModes = "available_modes"
            case availableCommands = "available_commands"
        }
    }
    private struct MessagePayload: Decodable { let role: Role; let text: String }
    private struct ToolCallPayload: Decodable {
        let id: String; let title: String; let kind: String
        let status: ToolStatus; let locations: [String]
    }
    private struct ToolUpdatePayload: Decodable {
        let id: String; let status: ToolStatus; let content: String?; let diff: Diff?
    }
    private struct PlanPayload: Decodable { let entries: [PlanEntry] }
    private struct PermissionPayload: Decodable {
        let id: String; let toolCall: String; let options: [PermissionOption]
        enum CodingKeys: String, CodingKey { case id, options, toolCall = "tool_call" }
    }
    private struct ResolvedPayload: Decodable { let id: String; let chosen: String }
    private struct ModeSetPayload: Decodable {
        let agentMode: String
        enum CodingKeys: String, CodingKey { case agentMode = "agent_mode" }
    }
    private struct TurnEndedPayload: Decodable { let reason: String }
    private struct GapPayload: Decodable { let reason: GapReason }
}
