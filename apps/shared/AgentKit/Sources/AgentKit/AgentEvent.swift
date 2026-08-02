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

/// One selectable option: a mode, or a model.
///
/// Carries the human `name` as well as the `id`. A picker built from ids alone
/// offers `acceptEdits` and `bypassPermissions` — the wire's words, not anyone's.
public struct AgentChoice: Decodable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let description: String

    public init(id: String, name: String, description: String = "") {
        self.id = id
        self.name = name
        self.description = description
    }
}

/// One thing a user can change about a session.
///
/// ACP's stabilised generic form. The client renders a control per option
/// rather than knowing in advance that "mode" and "model" exist — which is how
/// a subagent picker and a thought level arrive with no new code.
public struct ConfigOption: Decodable, Sendable, Equatable, Identifiable {
    public let id: String
    public let name: String
    public let description: String
    /// `mode`, `model`, `model_config`, `thought_level`, or empty. A hint for
    /// ordering only — never a reason to special-case one.
    public let category: String
    /// `select` or `boolean`.
    public let kind: String
    /// An option id for a select; `"true"`/`"false"` for a boolean.
    public let currentValue: String
    public let options: [AgentChoice]

    enum CodingKeys: String, CodingKey {
        case id, name, description, category, kind, options
        case currentValue = "current_value"
    }

    public init(
        id: String, name: String, description: String, category: String, kind: String,
        currentValue: String, options: [AgentChoice]
    ) {
        self.id = id
        self.name = name
        self.description = description
        self.category = category
        self.kind = kind
        self.currentValue = currentValue
        self.options = options
    }

    public var isBoolean: Bool { kind == "boolean" }
    public var isOn: Bool { currentValue == "true" }
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
    case sessionStarted(
        sessionID: String, agentMode: String?, availableModes: [AgentChoice],
        model: String?, availableModels: [AgentChoice], configOptions: [ConfigOption],
        availableCommands: [String])
    case message(role: Role, text: String)
    case toolCall(id: String, title: String, kind: String, status: ToolStatus, locations: [String])
    case toolUpdate(
        id: String, status: ToolStatus, title: String?, content: String?, diff: Diff?,
        locations: [String])
    case plan(entries: [PlanEntry])
    case permission(id: String, toolCall: String, options: [PermissionOption])
    case resolved(id: String, chosen: String)
    case modeSet(agentMode: String)
    /// A selector changed — by the user, or by the agent itself.
    case configSet(id: String, value: String)
    /// Context-window usage, resent as a turn consumes it.
    case usage(used: UInt64, size: UInt64)
    /// What this conversation is called, as the agent names it.
    case sessionInfo(title: String)
    /// The slash-command menu, resent once per turn. Feeds the `/` picker.
    case commandsAvailable(commands: [String])
    case turnEnded(reason: String)
    /// Everything written but not yet sent, in order. Sent whole on any change.
    case promptQueue(items: [QueuedPrompt])
    case gap(GapReason)
}

/// A prompt waiting for the current turn to end.
///
/// Overnight holds these rather than handing them straight to the agent, which
/// is the only reason one can still be rewritten or taken back — a prompt the
/// adapter has is gone.
public struct QueuedPrompt: Decodable, Sendable, Equatable, Identifiable {
    public let id: String
    public var text: String
    /// How many pictures are waiting with it.
    ///
    /// Decoded because a message can be an image and nothing else, and a queued
    /// row that showed only `text` then rendered an empty bubble — which is
    /// indistinguishable from the attachment having been dropped on the floor.
    public var images: [QueuedImage]?

    public var imageCount: Int { images?.count ?? 0 }

    public init(id: String, text: String, images: [QueuedImage]? = nil) {
        self.id = id
        self.text = text
        self.images = images
    }
}

/// Only its presence matters here — the bytes are the daemon's business.
public struct QueuedImage: Decodable, Sendable, Equatable {
    public let mime: String
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
                    availableModes: p.availableModes, model: p.model,
                    availableModels: p.availableModels,
                    configOptions: p.configOptions,
                    availableCommands: p.availableCommands)
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
                event = .toolUpdate(
                    id: p.id, status: p.status, title: p.title, content: p.content,
                    diff: p.diff, locations: p.locations)
            case "Plan":
                let p = try outer.decode(PlanPayload.self, forKey: key)
                event = .plan(entries: p.entries)
            case "Permission":
                let p = try outer.decode(PermissionPayload.self, forKey: key)
                event = .permission(id: p.id, toolCall: p.toolCall, options: p.options)
            case "Resolved":
                let p = try outer.decode(ResolvedPayload.self, forKey: key)
                event = .resolved(id: p.id, chosen: p.chosen)
            case "CommandsAvailable":
                let p = try outer.decode(CommandsAvailablePayload.self, forKey: key)
                event = .commandsAvailable(commands: p.commands)
            case "SessionInfo":
                let p = try outer.decode(SessionInfoPayload.self, forKey: key)
                event = .sessionInfo(title: p.title)
            case "Usage":
                let p = try outer.decode(UsagePayload.self, forKey: key)
                event = .usage(used: p.used, size: p.size)
            case "ConfigSet":
                let p = try outer.decode(ConfigSetPayload.self, forKey: key)
                event = .configSet(id: p.id, value: p.value)
            case "ModeSet":
                let p = try outer.decode(ModeSetPayload.self, forKey: key)
                event = .modeSet(agentMode: p.agentMode)
            case "PromptQueue":
                let p = try outer.decode(PromptQueuePayload.self, forKey: key)
                event = .promptQueue(items: p.items)
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
        let availableModes: [AgentChoice]
        let model: String?
        let availableModels: [AgentChoice]
        let configOptions: [ConfigOption]
        let availableCommands: [String]
        enum CodingKeys: String, CodingKey {
            case sessionID = "session_id"
            case agentMode = "agent_mode"
            case availableModes = "available_modes"
            case model
            case availableModels = "available_models"
            case configOptions = "config_options"
            case availableCommands = "available_commands"
        }
    }
    private struct MessagePayload: Decodable { let role: Role; let text: String }
    private struct ToolCallPayload: Decodable {
        let id: String; let title: String; let kind: String
        let status: ToolStatus; let locations: [String]
    }
    private struct ToolUpdatePayload: Decodable {
        let id: String
        let status: ToolStatus
        let title: String?
        let content: String?
        let diff: Diff?
        let locations: [String]
    }
    private struct PlanPayload: Decodable { let entries: [PlanEntry] }
    private struct PermissionPayload: Decodable {
        let id: String; let toolCall: String; let options: [PermissionOption]
        enum CodingKeys: String, CodingKey { case id, options, toolCall = "tool_call" }
    }
    private struct ResolvedPayload: Decodable { let id: String; let chosen: String }
    private struct CommandsAvailablePayload: Decodable { let commands: [String] }
    private struct ConfigSetPayload: Decodable { let id: String; let value: String }
    private struct UsagePayload: Decodable { let used: UInt64; let size: UInt64 }
    private struct SessionInfoPayload: Decodable { let title: String }
    private struct ModeSetPayload: Decodable {
        let agentMode: String
        enum CodingKeys: String, CodingKey { case agentMode = "agent_mode" }
    }
    private struct TurnEndedPayload: Decodable { let reason: String }
    private struct PromptQueuePayload: Decodable { let items: [QueuedPrompt] }
    private struct GapPayload: Decodable { let reason: GapReason }
}
