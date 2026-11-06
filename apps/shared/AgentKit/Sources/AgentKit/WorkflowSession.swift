import Foundation
import CryptoKit

public struct RetainedWorkflow: Codable, Sendable, Identifiable {
    public let id: String
    public let item: String
    public let run: String
    public let label: String
    public var reinspect = false
}

// One shared Apple adapter. Call resolves the current host Session on each
// invocation; this object owns only observations and an app-private journal.
@MainActor
public final class WorkflowSessionAdapter {
    public struct Current {
        public let account: String
        public let host: String
        public let connection: String
        public let call: @MainActor (String, String) async throws -> String
        public init(account: String, host: String, connection: String,
                    call: @escaping @MainActor (String, String) async throws -> String) {
            self.account = account; self.host = host; self.connection = connection; self.call = call
        }
    }
    private struct Observation { let revision: UInt64; let epoch: UInt64; let run: String }
    private enum Phase: String, Codable { case preparing, prepared, complete, abandoned }
    private struct Journal: Codable { let schema: Int; let payload: String; let checksum: String }
    private struct Entry: Codable {
        var phase: Phase = .preparing
        let retained: RetainedWorkflow
        let action: String
        var prepared: String?
        var attempts = 0
        var result: String?
    }
    private struct Snapshot: Decodable { let host: String; let presentation: WorkflowPresentation }
    private let root: URL
    private let current: @MainActor (String) -> Current?
    private var observations: [String: Observation] = [:]
    private var busy: Set<String> = []
    public init(root: URL, current: @escaping @MainActor (String) -> Current?) {
        self.root = root; self.current = current
    }
    public static func journalRoot() -> URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent(Bundle.main.bundleIdentifier ?? "FarCooler", isDirectory: true)
            .appendingPathComponent("WorkflowIntents", isDirectory: true)
    }
    private func json<T: Encodable>(_ value: T) throws -> String {
        let encoder = JSONEncoder(); encoder.outputFormatting = [.sortedKeys]
        return String(decoding: try encoder.encode(value), as: UTF8.self)
    }
    private func key(_ parts: [String]) throws -> String { try json(parts) }
    private func scope(_ host: String, _ workspace: String) throws -> (Current, String) {
        guard let c = current(host), !c.account.isEmpty, !c.host.isEmpty else { throw WorkflowDisplayError.pending }
        return (c, try key([c.account, c.host, workspace]))
    }
    private func url(_ scope: String) -> URL {
        root.appendingPathComponent(SHA256.hash(data: Data(scope.utf8)).map { String(format: "%02x", $0) }.joined() + ".json")
    }
    private func checksum(_ payload: String) -> String {
        SHA256.hash(data: Data(payload.utf8)).map { String(format: "%02x", $0) }.joined()
    }
    private func load(_ scope: String) throws -> [Entry] {
        let path = url(scope)
        if !FileManager.default.fileExists(atPath: path.path) { return [] }
        let journal = try JSONDecoder().decode(Journal.self, from: Data(contentsOf: path))
        guard journal.schema == 1, checksum(journal.payload) == journal.checksum else { throw WorkflowDisplayError.invalid }
        let entries = try JSONDecoder().decode([Entry].self, from: Data(journal.payload.utf8))
        for entry in entries {
            guard entry.attempts >= 0 else { throw WorkflowDisplayError.invalid }
            switch entry.phase {
            case .preparing, .abandoned:
                guard entry.prepared == nil, entry.attempts == 0, entry.result == nil else { throw WorkflowDisplayError.invalid }
            case .prepared:
                guard entry.prepared != nil, entry.result == nil else { throw WorkflowDisplayError.invalid }
            case .complete:
                guard entry.result != nil else { throw WorkflowDisplayError.invalid }
            }
        }
        return entries
    }
    private func save(_ scope: String, _ entries: [Entry]) throws {
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let payload = try json(entries)
        let journal = Journal(schema: 1, payload: payload, checksum: checksum(payload))
        try Data(json(journal).utf8).write(to: url(scope), options: .atomic)
    }
    public func port(host: String) -> WorkflowPort? {
        guard let c = current(host) else { return nil }
        return WorkflowPort(id: c.connection, read: { [self] in try await read($0, $1) },
            dispatch: { [self] in try await dispatch($0, $1, $2, $3) },
            pending: { [self] host, workspace in
                let (_, scope) = try self.scope(host, workspace)
                return try load(scope).filter { $0.phase == .prepared || $0.phase == .preparing }.map {
                    var retained = $0.retained; retained.reinspect = $0.phase == .preparing; return retained
                }
            }, retry: { [self] in try await retry($0, $1, $2) })
    }
    private func read(_ host: String, _ workspace: String) async throws -> String {
        let (c, scope) = try scope(host, workspace)
        let raw = try await c.call("workflow.snapshot", json(["workspace": workspace]))
        let latest = try self.scope(host, workspace)
        guard latest.1 == scope, latest.0.connection == c.connection else { throw WorkflowDisplayError.pending }
        let snapshot = try JSONDecoder().decode(Snapshot.self, from: Data(raw.utf8))
        guard snapshot.host == c.host else { throw WorkflowDisplayError.pending }
        let view = snapshot.presentation
        guard view.workspace == workspace, view.schema == 2 else { throw WorkflowDisplayError.invalid }
        for run in view.runs { for action in run.actions {
            observations[try key([scope, run.item, WorkflowActionJSON.canonical(action.json())])] = Observation(revision: view.revision, epoch: view.epoch, run: run.run)
        }}
        // Encode the typed display DTO directly: UInt64 never goes through Double.
        // WorkflowPresentation is display-only Decodable; extract the subtree
        // with Foundation, whose integer NSNumber preserves UInt64.
        let object = try JSONSerialization.jsonObject(with: Data(raw.utf8)) as! [String: Any]
        return String(decoding: try JSONSerialization.data(withJSONObject: object["presentation"]!), as: UTF8.self)
    }
    private func dispatch(_ host: String, _ workspace: String, _ item: String, _ action: String) async throws -> String {
        let (c, scope) = try scope(host, workspace)
        guard busy.insert(scope).inserted else { throw WorkflowDisplayError.pending }
        defer { busy.remove(scope) }
        var entries = try load(scope)
        let canonical = try WorkflowActionJSON.canonical(action)
        if let prior = entries.first(where: { $0.phase != .abandoned && $0.retained.item == item && $0.action == canonical }) {
            if let result = prior.result { return result }
            throw WorkflowDisplayError.pending
        }
        guard let observed = observations[try key([scope, item, canonical])] else { throw WorkflowDisplayError.pending }
        entries.append(Entry(retained: RetainedWorkflow(id: UUID().uuidString, item: item, run: observed.run,
            label: try JSONDecoder().decode(WorkflowAction.self, from: Data(action.utf8)).label), action: canonical))
        let index = entries.count - 1
        try save(scope, entries) // preparing marker survives loss of prepare reply
        let args = "{\"workspace\":\(try json(workspace)),\"item\":\(try json(item)),\"action\":\(canonical),\"revision\":\(try json(String(observed.revision))),\"epoch\":\(try json(String(observed.epoch)))}"
        let raw = try await c.call("workflow.prepare_selection", args)
        guard try self.scope(host, workspace).1 == scope else { throw WorkflowDisplayError.pending }
        guard let reply = try JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any] else { throw WorkflowDisplayError.invalid }
        if reply["kind"] as? String == "Inspection", let intent = reply["intent"] {
            let result = String(decoding: try JSONSerialization.data(withJSONObject: intent), as: UTF8.self)
            entries[index].phase = .complete; entries[index].result = result; try save(scope, entries); return result
        }
        guard reply["kind"] as? String == "Mutation", let prepared = reply["prepared"] else { throw WorkflowDisplayError.invalid }
        entries[index].phase = .prepared
        entries[index].prepared = String(decoding: try JSONSerialization.data(withJSONObject: prepared, options: .sortedKeys), as: UTF8.self)
        try save(scope, entries)
        return try await submit(host, workspace, scope, &entries, index)
    }
    private func retry(_ host: String, _ workspace: String, _ id: String) async throws -> String {
        let (_, scope) = try scope(host, workspace)
        guard busy.insert(scope).inserted else { throw WorkflowDisplayError.pending }
        defer { busy.remove(scope) }
        var entries = try load(scope)
        guard let index = entries.firstIndex(where: { $0.retained.id == id }), entries[index].result == nil else { throw WorkflowDisplayError.pending }
        if entries[index].phase == .preparing, entries[index].prepared == nil, entries[index].attempts == 0 {
            _ = try await read(host, workspace)
            guard try self.scope(host, workspace).1 == scope else { throw WorkflowDisplayError.pending }
            entries[index].phase = .abandoned; try save(scope, entries)
            return "No action was submitted. Inspect the refreshed run and choose an action."
        }
        guard entries[index].phase == .prepared else { throw WorkflowDisplayError.pending }
        return try await submit(host, workspace, scope, &entries, index)
    }
    private func submit(_ host: String, _ workspace: String, _ scope: String, _ entries: inout [Entry], _ index: Int) async throws -> String {
        let (c, now) = try self.scope(host, workspace)
        guard now == scope, let prepared = entries[index].prepared, entries[index].attempts < Int.max else { throw WorkflowDisplayError.pending }
        entries[index].attempts += 1; try save(scope, entries)
        let result = try await c.call("workflow.submit_action", "{\"prepared\":\(prepared)}")
        entries[index].phase = .complete; entries[index].result = result; try save(scope, entries)
        guard try self.scope(host, workspace).1 == scope else { throw WorkflowDisplayError.pending }
        return result
    }
}
private enum WorkflowActionJSON {
    static func canonical(_ raw: String) throws -> String {
        let object = try JSONSerialization.jsonObject(with: Data(raw.utf8))
        return String(decoding: try JSONSerialization.data(withJSONObject: object, options: .sortedKeys), as: UTF8.self)
    }
}
