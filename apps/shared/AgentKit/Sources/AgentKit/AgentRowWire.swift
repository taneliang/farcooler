import Foundation

/// One row of a terminal-mode agent's session, as the runner's projector
/// folds it (`crates/core/src/session_log/projector/rows.rs`, ov-363) and
/// `agent.rows` serves it (ov-366).
///
/// The id never changes once handed out, so a client applies a change to the
/// row it already drew rather than re-diffing the list. `ord` is the row's
/// index at insertion and `rev` the projection's revision when it last
/// changed; both only grow.
///
/// Decoded from the shape serde gives the Rust types: snake_case fields, and
/// every enum externally tagged (`{"Tool": {...}}`, `"Running"`,
/// `{"Failed": {"detail": "..."}}`). A kind or a state this build doesn't
/// know decodes to `.unknown` rather than failing the page, so a runner newer
/// than the app still draws every row it can.
public struct AgentRow: Sendable, Equatable, Identifiable, Codable {
    public var id: String
    public var ord: UInt64
    public var rev: UInt64
    /// The `Turn` row this one belongs to.
    public var turn: String?
    /// Announced by a hook and not yet confirmed by a transcript record.
    public var provisional: Bool
    public var kind: Kind

    public init(id: String, ord: UInt64, rev: UInt64, turn: String? = nil, provisional: Bool = false, kind: Kind) {
        self.id = id
        self.ord = ord
        self.rev = rev
        self.turn = turn
        self.provisional = provisional
        self.kind = kind
    }

    public enum Kind: Sendable, Equatable, Codable {
        case turn(Turn)
        case prose(Prose)
        case thinking(Thinking)
        case tool(Tool)
        case subagent(Subagent)
        case ask(Ask)
        case queued(Queued)
        case notice(Notice)
        case handoff(Handoff)
        case gap(Gap)
        /// A kind this build doesn't know, by its name.
        case unknown(String)
    }

    /// One prompt and everything the agent did about it.
    public struct Turn: Sendable, Equatable, Codable {
        public var prompt: String
        /// `Typed`, `Queued`, `Notification`, `Sdk`, `System` or `Other`.
        public var origin: String
        public var startedMs: Int64?
        public var endedMs: Int64?
        public var durationMs: Int64?
        /// Nil while the turn is open.
        public var outcome: Outcome?
        public var backgroundRunning: Int
        /// `Busy`, `Idle` or `Shell`, on the newest turn only.
        public var activity: String?

        public enum Outcome: Sendable, Equatable, Codable {
            case finished, interrupted, unrecorded
            case failed(String)
            case other(String)
        }
    }

    public struct Prose: Sendable, Equatable, Codable {
        public var text: String
        /// The turn's closing answer rather than narration on the way there.
        public var conclusion: Bool
        public var atMs: Int64?
    }

    public struct Thinking: Sendable, Equatable, Codable {
        public var startedMs: Int64?
        public var endedMs: Int64?
    }

    public struct Tool: Sendable, Equatable, Codable {
        public var name: String
        public var summary: String
        public var status: Status
        public var startedMs: Int64?
        public var endedMs: Int64?
        public var diff: [Hunk]
        public var filePath: String?
    }

    /// A tool's or a subagent's state, folded to what a row draws.
    public enum Status: Sendable, Equatable, Codable {
        case running, done, failed
        /// A subagent's own ending other than done or failed: `Killed`,
        /// `Stopped`, or a word this build doesn't know.
        case ended(String)
    }

    public struct Hunk: Sendable, Equatable, Codable {
        public var oldStart: Int
        public var oldLines: Int
        public var newStart: Int
        public var newLines: Int
        /// Unified-diff lines, each starting with ` `, `-` or `+`.
        public var lines: [String]
    }

    public struct Subagent: Sendable, Equatable, Codable {
        public var toolUseId: String
        public var agentType: String
        public var description: String
        public var background: Bool
        public var status: Status
        public var startedMs: Int64?
        public var endedMs: Int64?
        public var toolCount: Int
        /// Its latest tool call, as `Name summary`.
        public var currentAction: String
        public var lastMs: Int64?
    }

    public struct Ask: Sendable, Equatable, Codable {
        /// `Question`, `Permission` or `PlanExit`.
        public var kind: String
        public var text: String
        public var tool: String?
        public var askedMs: Int64?
        public var answered: Bool
    }

    public struct Queued: Sendable, Equatable, Codable {
        public var text: String
        /// `Waiting`, `Sent` or `Withdrawn`.
        public var state: String
        public var atMs: Int64?
    }

    public struct Notice: Sendable, Equatable, Codable {
        /// `Compacted`, `Cleared`, `Resumed`, `Command` or `ApiError`.
        public var kind: String
        public var text: String
        public var atMs: Int64?
    }

    public struct Handoff: Sendable, Equatable, Codable {
        public var reason: String
        public var atMs: Int64?
    }

    public struct Gap: Sendable, Equatable, Codable {
        /// `Unparsed`, `TooLarge`, `Rewritten`, or `Unknown <name>`.
        public var reason: String
        public var count: Int
    }
}

/// A page of rows: `agent.rows {terminal, before?, limit?}`'s answer as the
/// client core spells it (`crates/client/src/ffi/rows_args.rs`).
public struct AgentRowPage: Sendable, Equatable, Codable {
    public var epoch: UInt64
    public var rev: UInt64
    public var moreBefore: Bool
    /// Oldest first.
    public var rows: [AgentRow]

    public init(epoch: UInt64, rev: UInt64, moreBefore: Bool, rows: [AgentRow]) {
        self.epoch = epoch
        self.rev = rev
        self.moreBefore = moreBefore
        self.rows = rows
    }
}

/// What changed after a revision: `agent.rows_follow`'s answer.
public struct AgentRowChanges: Sendable, Equatable, Codable {
    public var epoch: UInt64
    public var rev: UInt64
    /// The runner can't say what changed (a new epoch, or too much at once):
    /// page again.
    public var reset: Bool
    public var changes: [Change]

    public enum Change: Sendable, Equatable, Codable {
        case insert(AgentRow)
        case update(AgentRow)
        case remove(id: String, rev: UInt64)
    }

    public init(epoch: UInt64, rev: UInt64, reset: Bool, changes: [Change]) {
        self.epoch = epoch
        self.rev = rev
        self.reset = reset
        self.changes = changes
    }
}

/// Why a page or a follow couldn't be read.
public struct AgentRowDecodeError: Error, Equatable {
    public var what: String
}

// MARK: - Decoding

extension AgentRowPage {
    /// The JSON the client core answers `agent.rows` with.
    public static func decode(_ data: Data) throws -> AgentRowPage {
        let object = try AgentRowJSON.object(data)
        let rows = (object["rows"] as? [Any] ?? []).compactMap { ($0 as? [String: Any]).flatMap(AgentRow.init(json:)) }
        return AgentRowPage(
            epoch: AgentRowJSON.uint(object["epoch"]), rev: AgentRowJSON.uint(object["rev"]),
            moreBefore: object["moreBefore"] as? Bool ?? false, rows: rows)
    }
}

extension AgentRowChanges {
    /// The JSON the client core answers `agent.rows_follow` with.
    public static func decode(_ data: Data) throws -> AgentRowChanges {
        let object = try AgentRowJSON.object(data)
        let changes: [Change] = (object["changes"] as? [Any] ?? []).compactMap { item in
            guard let change = item as? [String: Any], let id = change["id"] as? String else { return nil }
            let rev = AgentRowJSON.uint(change["rev"])
            switch change["kind"] as? String {
            case "remove": return .remove(id: id, rev: rev)
            case "insert": return (change["row"] as? [String: Any]).flatMap(AgentRow.init(json:)).map(Change.insert)
            default: return (change["row"] as? [String: Any]).flatMap(AgentRow.init(json:)).map(Change.update)
            }
        }
        return AgentRowChanges(
            epoch: AgentRowJSON.uint(object["epoch"]), rev: AgentRowJSON.uint(object["rev"]),
            reset: object["reset"] as? Bool ?? false, changes: changes)
    }
}

/// The wire is read with `JSONSerialization` rather than `Codable`: every
/// enum on it is externally tagged, and a tolerant hand reader is shorter
/// than ten custom `init(from:)`s and as fast. The types' synthesized
/// `Codable` is the cache's own shape (`AgentRowCache`), not the wire's.
enum AgentRowJSON {
    static func object(_ data: Data) throws -> [String: Any] {
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw AgentRowDecodeError(what: "not an object")
        }
        return object
    }

    static func uint(_ value: Any?) -> UInt64 {
        (value as? NSNumber)?.uint64Value ?? 0
    }

    static func int(_ value: Any?) -> Int {
        (value as? NSNumber)?.intValue ?? 0
    }

    static func ms(_ value: Any?) -> Int64? {
        (value as? NSNumber)?.int64Value
    }

    /// An externally tagged enum: `"Name"` or `{"Name": payload}`.
    static func tag(_ value: Any?) -> (name: String, payload: Any?)? {
        if let name = value as? String { return (name, nil) }
        if let object = value as? [String: Any], let (name, payload) = object.first { return (name, payload) }
        return nil
    }
}

extension AgentRow {
    /// One serialized `projector::Row`, or nil when it has no id.
    public init?(json: [String: Any]) {
        guard let id = json["id"] as? String else { return nil }
        self.id = id
        ord = AgentRowJSON.uint(json["ord"])
        rev = AgentRowJSON.uint(json["rev"])
        turn = json["turn"] as? String
        provisional = json["provisional"] as? Bool ?? false
        kind = Self.kind(json["kind"])
    }

    private static func kind(_ value: Any?) -> Kind {
        guard let (name, payload) = AgentRowJSON.tag(value) else { return .unknown("") }
        let p = payload as? [String: Any] ?? [:]
        let text = { (key: String) in p[key] as? String ?? "" }
        switch name {
        case "Turn":
            let outcome: Turn.Outcome? = AgentRowJSON.tag(p["outcome"]).map { tag in
                switch tag.name {
                case "Finished": .finished
                case "Interrupted": .interrupted
                case "Unrecorded": .unrecorded
                case "Failed": .failed((tag.payload as? [String: Any])?["detail"] as? String ?? "")
                default: .other(tag.name)
                }
            }
            return .turn(Turn(
                prompt: text("prompt"), origin: AgentRowJSON.tag(p["origin"])?.name ?? "Other",
                startedMs: AgentRowJSON.ms(p["started_ms"]), endedMs: AgentRowJSON.ms(p["ended_ms"]),
                durationMs: AgentRowJSON.ms(p["duration_ms"]), outcome: outcome,
                backgroundRunning: AgentRowJSON.int(p["background_running"]),
                activity: AgentRowJSON.tag(p["activity"])?.name))
        case "Prose":
            return .prose(Prose(text: text("text"), conclusion: p["conclusion"] as? Bool ?? false, atMs: AgentRowJSON.ms(p["at_ms"])))
        case "Thinking":
            return .thinking(Thinking(startedMs: AgentRowJSON.ms(p["started_ms"]), endedMs: AgentRowJSON.ms(p["ended_ms"])))
        case "Tool":
            let hunks = (p["diff"] as? [Any] ?? []).compactMap { $0 as? [String: Any] }.map { h in
                Hunk(
                    oldStart: AgentRowJSON.int(h["old_start"]), oldLines: AgentRowJSON.int(h["old_lines"]),
                    newStart: AgentRowJSON.int(h["new_start"]), newLines: AgentRowJSON.int(h["new_lines"]),
                    lines: h["lines"] as? [String] ?? [])
            }
            return .tool(Tool(
                name: text("name"), summary: text("summary"), status: status(p["status"]),
                startedMs: AgentRowJSON.ms(p["started_ms"]), endedMs: AgentRowJSON.ms(p["ended_ms"]),
                diff: hunks, filePath: p["file_path"] as? String))
        case "Subagent":
            return .subagent(Subagent(
                toolUseId: text("tool_use_id"), agentType: text("agent_type"), description: text("description"),
                background: p["background"] as? Bool ?? false, status: status(p["status"]),
                startedMs: AgentRowJSON.ms(p["started_ms"]), endedMs: AgentRowJSON.ms(p["ended_ms"]),
                toolCount: AgentRowJSON.int(p["tool_count"]), currentAction: text("current_action"),
                lastMs: AgentRowJSON.ms(p["last_ms"])))
        case "Ask":
            return .ask(Ask(
                kind: AgentRowJSON.tag(p["kind"])?.name ?? "", text: text("text"), tool: p["tool"] as? String,
                askedMs: AgentRowJSON.ms(p["asked_ms"]), answered: p["answered"] as? Bool ?? false))
        case "Queued":
            return .queued(Queued(text: text("text"), state: AgentRowJSON.tag(p["state"])?.name ?? "Waiting", atMs: AgentRowJSON.ms(p["at_ms"])))
        case "Notice":
            return .notice(Notice(kind: AgentRowJSON.tag(p["kind"])?.name ?? "", text: text("text"), atMs: AgentRowJSON.ms(p["at_ms"])))
        case "Handoff":
            return .handoff(Handoff(reason: text("reason"), atMs: AgentRowJSON.ms(p["at_ms"])))
        case "Gap":
            let reason = AgentRowJSON.tag(p["reason"]).map { tag in
                (tag.payload as? String).map { "\(tag.name) \($0)" } ?? tag.name
            }
            return .gap(Gap(reason: reason ?? "", count: AgentRowJSON.int(p["count"])))
        default:
            return .unknown(name)
        }
    }

    private static func status(_ value: Any?) -> Status {
        switch AgentRowJSON.tag(value)?.name {
        case "Running": .running
        case "Done", "Completed": .done
        case "Failed": .failed
        case let other?: .ended(other)
        case nil: .ended("")
        }
    }
}
