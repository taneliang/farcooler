import Foundation

/// Where a window or a phone is, or should go (ov-182, ov-183).
///
/// One value for two jobs. **Restore**: "where I was", written by this device
/// in the ids it already holds (`Runner.host`, a task's id, a terminal id).
/// **Notification**: "what this is about", read off a push or a local post in
/// portable ids (`Runner.id`, a task's key and repository, a terminal id).
/// `DestinationResolver` turns either into a place that exists, waiting for
/// the runner and falling back a level at a time when a level is gone.
///
/// The wire form is JSON, `{"v":1, …}`, written with sorted keys and explicit
/// discriminators, never Swift's synthesized enum coding: Android's
/// `com.farcooler.model.Destination` reads and writes the same bytes, and
/// `test/fixtures/destinations.json` holds both suites to them.
public struct Destination: Hashable, Sendable {
    /// The runner, as much as is known of it. Empty for Needs You, and for
    /// an agent push from a runner too old to say which it is.
    public var runner: Runner
    public var place: Place
    /// The task tab, kept only while `place` is the task and it offers it.
    public var tab: Tab?
    /// The phone's workspace segment, kept only while `place` is a workspace.
    public var segment: Segment?
    /// The terminal the keyboard was in, or the pane a notice is about once
    /// resolved. Dropped quietly when the runner no longer has it.
    public var pane: String?
    /// The agent pane a task's Agent tab shows (the Mac's `chosenAgents`).
    public var agent: String?
    /// A decision or a question waiting on the task: open with it in front.
    /// Only meaningful on a task.
    public var question: Bool

    public init(
        runner: Runner = Runner(), place: Place, tab: Tab? = nil, segment: Segment? = nil,
        pane: String? = nil, agent: String? = nil, question: Bool = false
    ) {
        self.runner = runner
        self.place = place
        self.tab = tab
        self.segment = segment
        self.pane = pane
        self.agent = agent
        self.question = question
    }

    /// Needs You, with nothing else said.
    public static let needsYou = Destination(place: .needsYou)

    /// A runner, by either or both of its names.
    public struct Runner: Hashable, Sendable {
        /// This device's own handle for it: the Mac's `DaemonClient.target`
        /// (`""` is this Mac), the phone's `Host.id`. Never sent off the device
        /// except as a local post's own `target`.
        public var host: String?
        /// Its `Host.runner_id`, the portable name a push carries. Compared
        /// without case.
        public var id: String?

        public init(host: String? = nil, id: String? = nil) {
            self.host = host
            self.id = id
        }

        public var isEmpty: Bool { host == nil && id == nil }
    }

    /// A task, by its id when this device has read it, by its key (and the
    /// repository a key is unique within) when a notice names it. At least
    /// one of `id` and `key`.
    public struct TaskRef: Hashable, Sendable {
        public var id: String?
        public var key: String?
        public var repository: String?

        public init(id: String? = nil, key: String? = nil, repository: String? = nil) {
            self.id = id
            self.key = key
            self.repository = repository
        }
    }

    /// The level the destination is at. Its parents are `ancestors`.
    public enum Place: Hashable, Sendable {
        case needsYou
        /// A workspace's board, as it opens by default.
        case workspace(String)
        /// A workspace's orchestrator.
        case orchestrator(workspace: String)
        /// A finished status's History page (`done`, `cancelled`).
        case history(workspace: String, status: String)
        /// A task. The workspace is nil when a notice names only a key.
        case task(workspace: String?, task: TaskRef)
        /// A worktree. A nil workspace is a loose (unclaimed) one, or one a
        /// notice didn't say.
        case worktree(String, workspace: String?)
        /// A pane, by terminal id, before it's resolved to its worktree.
        case terminal(String)
    }

    /// A task's tabs, as the Mac's `TaskTab` names them. Files is the
    /// Mac's (ov-189); a phone, which has no such tab, reads it as none.
    public enum Tab: String, CaseIterable, Hashable, Sendable {
        case overview, agent, changes, files
    }

    /// A phone workspace screen's segments, as `WorkspaceSegment` names them.
    public enum Segment: String, CaseIterable, Hashable, Sendable {
        case orchestrator, board, worktrees
    }
}

// MARK: - The ladder

extension Destination.Place {
    /// The workspace this place is in, when it says.
    public var workspace: String? {
        switch self {
        case .needsYou, .terminal: nil
        case .workspace(let id): id
        case .orchestrator(let id), .history(let id, _): id
        case .task(let id, _), .worktree(_, let id): id
        }
    }

    /// The places to fall back to when this one is gone, nearest first, as
    /// far as the place itself knows. A task, a worktree, an orchestrator and
    /// a History page fall back to their workspace; a terminal knows no
    /// parent until it's resolved. The runner's home, the last workspace and
    /// Needs You come after these, and are `DestinationResolver`'s to add.
    public var ancestors: [Destination.Place] {
        switch self {
        case .needsYou, .workspace, .terminal: []
        case .orchestrator(let id), .history(let id, _): [.workspace(id)]
        case .task(let id, _), .worktree(_, let id): id.map { [.workspace($0)] } ?? []
        }
    }
}

// MARK: - The wire form

extension Destination {
    /// The encoding's version. A value of any other version decodes as nil.
    public static let version = 1

    /// The JSON object, as nested dictionaries and strings.
    public var json: [String: Any] {
        var out: [String: Any] = ["v": Self.version, "place": Self.json(place)]
        var runnerObject: [String: Any] = [:]
        if let host = runner.host { runnerObject["host"] = host }
        if let id = runner.id { runnerObject["id"] = id }
        if !runnerObject.isEmpty { out["runner"] = runnerObject }
        if let tab { out["tab"] = tab.rawValue }
        if let segment { out["segment"] = segment.rawValue }
        if let pane { out["pane"] = pane }
        if let agent { out["agent"] = agent }
        if question { out["question"] = true }
        return out
    }

    private static func json(_ place: Place) -> [String: Any] {
        switch place {
        case .needsYou:
            return ["kind": "needs-you"]
        case .workspace(let id):
            return ["kind": "workspace", "workspace": id]
        case .orchestrator(let id):
            return ["kind": "orchestrator", "workspace": id]
        case .history(let id, let status):
            return ["kind": "history", "workspace": id, "status": status]
        case .task(let workspace, let task):
            var ref: [String: Any] = [:]
            if let id = task.id { ref["id"] = id }
            if let key = task.key { ref["key"] = key }
            if let repository = task.repository { ref["repository"] = repository }
            var out: [String: Any] = ["kind": "task", "task": ref]
            if let workspace { out["workspace"] = workspace }
            return out
        case .worktree(let id, let workspace):
            var out: [String: Any] = ["kind": "worktree", "worktree": id]
            if let workspace { out["workspace"] = workspace }
            return out
        case .terminal(let id):
            return ["kind": "terminal", "terminal": id]
        }
    }

    /// The encoding as a string: compact, keys sorted, `/` not escaped, so
    /// it is the same bytes Android writes.
    public var encoded: String {
        let data = try? JSONSerialization.data(
            withJSONObject: json, options: [.sortedKeys, .withoutEscapingSlashes])
        return data.flatMap { String(data: $0, encoding: .utf8) } ?? ""
    }

    /// A destination from its encoding, or nil: a version this build doesn't
    /// write, a place kind it doesn't know, or a place missing what it needs
    /// is no place to go, not a partial one. A tab or a segment this build
    /// doesn't know is dropped alone; keys it doesn't know are ignored, so a
    /// later build can add an optional field without a new version.
    public init?(encoded: String) {
        guard let object = try? JSONSerialization.jsonObject(with: Data(encoded.utf8)) as? [String: Any]
        else { return nil }
        self.init(json: object)
    }

    public init?(json object: [String: Any]) {
        guard Self.int(object["v"]) == Self.version,
            let placeObject = object["place"] as? [String: Any],
            let place = Self.place(placeObject)
        else { return nil }
        let runnerObject = object["runner"] as? [String: Any] ?? [:]
        self.init(
            runner: Runner(host: runnerObject["host"] as? String, id: Self.nonEmpty(runnerObject["id"])),
            place: place,
            tab: (object["tab"] as? String).flatMap(Tab.init(rawValue:)),
            segment: (object["segment"] as? String).flatMap(Segment.init(rawValue:)),
            pane: Self.nonEmpty(object["pane"]),
            agent: Self.nonEmpty(object["agent"]),
            question: Self.bool(object["question"]) ?? false)
    }

    private static func place(_ object: [String: Any]) -> Place? {
        let workspace = nonEmpty(object["workspace"])
        switch object["kind"] as? String {
        case "needs-you":
            return .needsYou
        case "workspace":
            return workspace.map(Place.workspace)
        case "orchestrator":
            return workspace.map { .orchestrator(workspace: $0) }
        case "history":
            guard let workspace, let status = nonEmpty(object["status"]) else { return nil }
            return .history(workspace: workspace, status: status)
        case "task":
            guard let ref = object["task"] as? [String: Any] else { return nil }
            let task = TaskRef(id: nonEmpty(ref["id"]), key: nonEmpty(ref["key"]), repository: nonEmpty(ref["repository"]))
            guard task.id != nil || task.key != nil else { return nil }
            return .task(workspace: workspace, task: task)
        case "worktree":
            return nonEmpty(object["worktree"]).map { .worktree($0, workspace: workspace) }
        case "terminal":
            return nonEmpty(object["terminal"]).map(Place.terminal)
        default:
            return nil
        }
    }

    /// A non-empty string, or nil: an empty id names nothing.
    static func nonEmpty(_ value: Any?) -> String? {
        (value as? String).flatMap { $0.isEmpty ? nil : $0 }
    }

    /// An integer from JSON, which `JSONSerialization` hands over as a number:
    /// not a boolean, and not `1.0`, which Kotlin's reader refuses too.
    private static func int(_ value: Any?) -> Int? {
        guard let number = value as? NSNumber, CFGetTypeID(number) != CFBooleanGetTypeID(),
            !CFNumberIsFloatType(number)
        else { return nil }
        return number.intValue
    }

    /// A JSON boolean, and only that: `1` would bridge to `true` here and
    /// read as false on Android.
    private static func bool(_ value: Any?) -> Bool? {
        guard let number = value as? NSNumber, CFGetTypeID(number) == CFBooleanGetTypeID() else { return nil }
        return number.boolValue
    }
}

extension Destination: Codable {
    /// Coded as its encoding, a string, so a `Codable` container (a saved
    /// `@SceneStorage`, a `UserDefaults` blob) holds the same bytes a
    /// notification carries.
    public init(from decoder: Decoder) throws {
        let text = try decoder.singleValueContainer().decode(String.self)
        guard let value = Destination(encoded: text) else {
            throw DecodingError.dataCorrupted(
                .init(codingPath: decoder.codingPath, debugDescription: "Not a destination this build reads."))
        }
        self = value
    }

    public func encode(to encoder: Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(encoded)
    }
}
