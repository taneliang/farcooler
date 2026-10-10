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
        /// claude's generic `Try "…"` example (ov-409): not part of the
        /// conversation, shown as the composer's placeholder.
        case hint(Hint)
        /// The agent's task list as a turn left it (ov-452): one checklist
        /// row a turn, changed in place.
        case tasks(Tasks)
        /// A kind this build doesn't know, by its name.
        case unknown(String)
    }

    /// One prompt and everything the agent did about it.
    public struct Turn: Sendable, Equatable, Codable {
        public var prompt: String
        /// `Typed`, `Queued`, `Notification`, `Sdk`, `System`, `Scheduled`
        /// (a scheduled task firing, ov-452) or `Other`.
        public var origin: String
        public var startedMs: Int64?
        public var endedMs: Int64?
        public var durationMs: Int64?
        /// Nil while the turn is open.
        public var outcome: Outcome?
        public var backgroundRunning: Int
        /// `Busy`, `Idle`, `Shell` or `Waiting` (a dialog up, ov-368), on the
        /// newest turn only.
        public var activity: String?
        /// The next prompt claude's empty box suggests, on the newest turn
        /// only, once the agent rests (ov-409). A draft the person may take,
        /// never something to send.
        public var suggestion: String?
        /// The images the prompt carried, in order, by type (ov-454): pasted
        /// in the terminal or sent from a composer. Their bytes come one at a
        /// time through `agent.image`, by this row's id and the index here.
        public var images: [PromptImage] = []
        /// The tokens main's newest model call used, context and answer
        /// together (ov-453). Optional for the cache's sake; nil until a call
        /// says.
        public var tokens: Int? = nil

        public enum Outcome: Sendable, Equatable, Codable {
            case finished, interrupted, unrecorded
            case failed(String)
            case other(String)
        }
    }

    /// One image a prompt carried: its MIME type, as the transcript says it.
    public struct PromptImage: Sendable, Equatable, Codable, Hashable {
        public var mime: String
        public init(mime: String) { self.mime = mime }
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
        /// What it was called with, a `key: value` line per field, cut short
        /// by the runner (ov-452). Optional for the cache's sake.
        public var input: String? = nil
        /// What it answered, cut likewise; nil until it does.
        public var result: String? = nil

        /// Whether it has anything to open to.
        public var opens: Bool { input != nil || result != nil || !diff.isEmpty }
    }

    public struct Tasks: Sendable, Equatable, Codable {
        public var items: [TaskItem]

        public init(items: [TaskItem]) { self.items = items }
    }

    public struct TaskItem: Sendable, Equatable, Codable {
        public var subject: String
        /// `Pending`, `InProgress` or `Completed`.
        public var status: String

        public init(subject: String, status: String) {
            self.subject = subject
            self.status = status
        }
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
        /// Its `agentId`, once the runner knows it: what its own rows are
        /// asked for by (ov-453). Optional for the cache's sake.
        public var agentId: String? = nil
        /// The tokens its newest model call used, as claude's agent panel
        /// counts them (ov-453); nil until its transcript says.
        public var tokens: Int? = nil
    }

    public struct Ask: Sendable, Equatable, Codable {
        /// `Question`, `Permission` or `PlanExit`.
        public var kind: String
        public var text: String
        public var tool: String?
        public var askedMs: Int64?
        public var answered: Bool
        /// What `terminal.agent_answer` takes while the runner's hook holds
        /// the ask (ov-370); nil once the hold ends, when only the terminal
        /// can answer it.
        public var held: String? = nil
        /// A question's questions, whole. Optional only for the cache's sake:
        /// a row cached before them decodes. `questionList` reads it.
        public var questions: [Question]? = nil
        /// A plan's text, its line breaks kept.
        public var plan: String? = nil
        /// The device whose answer the hook took: "iPhone", "Mac".
        public var answeredBy: String? = nil

        public var questionList: [Question] { questions ?? [] }

        public struct Question: Sendable, Equatable, Codable {
            /// The words claude asked, which its answer is keyed by.
            public var question: String
            /// Claude's short label for it: "Color".
            public var header: String
            public var options: [Option]
            /// Several may be chosen; claude reads them joined by ", ".
            public var multiSelect: Bool

            public init(question: String, header: String, options: [Option], multiSelect: Bool) {
                self.question = question
                self.header = header
                self.options = options
                self.multiSelect = multiSelect
            }
        }

        public struct Option: Sendable, Equatable, Codable {
            public var label: String
            public var description: String

            public init(label: String, description: String) {
                self.label = label
                self.description = description
            }
        }
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

    /// The id of the one `Hint` row a session has.
    public static let hintID = "hint:composer"

    public struct Hint: Sendable, Equatable, Codable {
        /// Empty once claude's box shows something else.
        public var text: String
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
                activity: AgentRowJSON.tag(p["activity"])?.name,
                suggestion: (p["suggestion"] as? String).flatMap { $0.isEmpty ? nil : $0 },
                images: (p["images"] as? [Any] ?? []).compactMap { $0 as? [String: Any] }.map { PromptImage(mime: $0["mime"] as? String ?? "") },
                tokens: (p["tokens"] as? NSNumber)?.intValue))
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
                diff: hunks, filePath: p["file_path"] as? String,
                input: p["input"] as? String, result: p["result"] as? String))
        case "Subagent":
            return .subagent(Subagent(
                toolUseId: text("tool_use_id"), agentType: text("agent_type"), description: text("description"),
                background: p["background"] as? Bool ?? false, status: status(p["status"]),
                startedMs: AgentRowJSON.ms(p["started_ms"]), endedMs: AgentRowJSON.ms(p["ended_ms"]),
                toolCount: AgentRowJSON.int(p["tool_count"]), currentAction: text("current_action"),
                lastMs: AgentRowJSON.ms(p["last_ms"]), agentId: (p["agent_id"] as? String).flatMap { $0.isEmpty ? nil : $0 },
                tokens: (p["tokens"] as? NSNumber)?.intValue))
        case "Ask":
            let questions = (p["questions"] as? [Any] ?? []).compactMap { $0 as? [String: Any] }.map { q in
                Ask.Question(
                    question: q["question"] as? String ?? "", header: q["header"] as? String ?? "",
                    options: (q["options"] as? [Any] ?? []).compactMap { $0 as? [String: Any] }.map {
                        Ask.Option(label: $0["label"] as? String ?? "", description: $0["description"] as? String ?? "")
                    },
                    multiSelect: q["multi_select"] as? Bool ?? false)
            }
            return .ask(Ask(
                kind: AgentRowJSON.tag(p["kind"])?.name ?? "", text: text("text"), tool: p["tool"] as? String,
                askedMs: AgentRowJSON.ms(p["asked_ms"]), answered: p["answered"] as? Bool ?? false,
                held: p["held"] as? String, questions: questions.isEmpty ? nil : questions, plan: p["plan"] as? String,
                answeredBy: p["answered_by"] as? String))
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
        case "Hint":
            return .hint(Hint(text: text("text")))
        case "Tasks":
            let items = (p["items"] as? [Any] ?? []).compactMap { $0 as? [String: Any] }.map {
                TaskItem(subject: $0["subject"] as? String ?? "", status: AgentRowJSON.tag($0["status"])?.name ?? "Pending")
            }
            return .tasks(Tasks(items: items))
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
