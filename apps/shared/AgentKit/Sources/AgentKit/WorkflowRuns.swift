import Foundation
import Combine

// Display-only schema 2. Semantic IDs are strings, never native policy enums.
public struct WorkflowAction: Codable, Equatable, Sendable {
    private let descriptor: [String: String]
    public var kind: String { descriptor["kind"]! }
    public var label: String { descriptor["label"]! }
    public var guardValue: String { descriptor["guard"]! }
    public init(from decoder: any Decoder) throws {
        let container = try decoder.singleValueContainer()
        let value = try container.decode([String: String].self)
        guard value["kind"] != nil, value["label"] != nil, value["guard"] != nil else {
            throw WorkflowDisplayError.invalid
        }
        descriptor = value
    }
    public func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        try container.encode(descriptor)
    }
    public func json() throws -> String { String(decoding: try JSONEncoder().encode(self), as: UTF8.self) }
}
public struct WorkflowStage: Decodable, Sendable {
    public let id: String
    public let label: String
    public let progression_label: String
}
public struct WorkflowGate: Decodable, Sendable {
    public let label: String
    public let text: String
    public let principal: String
}
public struct WorkflowReason: Decodable, Sendable {
    public let kind: String
    public let label: String
    public let text: String
}
public struct WorkflowEvidence: Decodable, Sendable {
    public let candidate: String
    public let manifest: String
    public let receipt: String
    public let policy_pin: String
}
public struct WorkflowRun: Decodable, Identifiable, Sendable {
    public var id: String { item }
    public let label: String
    public let phase_label: String
    public let item: String
    public let run: String
    public let definition_pin: String
    public let policy_pin: String
    public let candidate: String?
    public let latest_evidence: WorkflowEvidence?
    public let pending_effect: String?
    public let iteration: UInt8
    public let attempt: UInt8
    public let fence: UInt64
    public let reworks_remaining: UInt8
    public let stages: [WorkflowStage]
    public let stage: WorkflowStage?
    public let gate: WorkflowGate?
    public let reasons: [WorkflowReason]
    public let actions: [WorkflowAction]
}
public struct WorkflowPresentation: Decodable, Sendable {
    public let schema: UInt32
    public let workspace: String
    public let epoch: UInt64
    public let revision: UInt64
    public let runs: [WorkflowRun]
    public static func decode(_ json: String, workspace: String) throws -> Self {
        let value = try JSONDecoder().decode(Self.self, from: Data(json.utf8))
        guard value.schema == 2, value.workspace == workspace else { throw WorkflowDisplayError.invalid }
        return value
    }
}
public enum WorkflowDisplayError: Error { case invalid, pending }

// ov-925 installs a getter over the current authenticated Session at the app
// root. The adapter re-reads State/auth and validates the opaque descriptor.
// Inspection text is a local identity/result slot; it never implies a fetch.
public struct WorkflowPort: Sendable {
    // Host rotates identity for each authenticated connection; nil on disconnect.
    public let id: String
    public let read: @MainActor @Sendable (String, String) async throws -> String
    public let dispatch: @MainActor @Sendable (String, String, String, String) async throws -> String
    public let pending: @MainActor @Sendable (String, String) async throws -> [RetainedWorkflow]
    public let retry: @MainActor @Sendable (String, String, String) async throws -> String
    public init(
        id: String,
        read: @escaping @MainActor @Sendable (String, String) async throws -> String,
        dispatch: @escaping @MainActor @Sendable (String, String, String, String) async throws -> String,
        pending: @escaping @MainActor @Sendable (String, String) async throws -> [RetainedWorkflow] = { _, _ in [] },
        retry: @escaping @MainActor @Sendable (String, String, String) async throws -> String = { _, _, _ in throw WorkflowDisplayError.pending }
    ) { self.id = id; self.read = read; self.dispatch = dispatch; self.pending = pending; self.retry = retry }
}

@MainActor
public final class WorkflowRunsModel: ObservableObject {
    public let host: String
    public let workspace: String
    @Published public private(set) var presentation: WorkflowPresentation?
    @Published public private(set) var retained: [RetainedWorkflow] = []
    @Published public private(set) var ready = false
    @Published public private(set) var busy = false
    @Published public private(set) var notice = ""
    @Published public private(set) var inspection = ""
    private var generation: UInt64 = 0
    private var selection: UInt64 = 0
    public init(host: String, workspace: String) { self.host = host; self.workspace = workspace }
    public func clearInspection() { selection &+= 1; inspection = "" }
    public func disconnect() {
        generation &+= 1; ready = false; busy = false; inspection = ""
        notice = "Reconnect to refresh workflow runs."
    }
    // A completion ticket must still match after the caller resumes from await.
    @discardableResult
    public func refresh(_ port: WorkflowPort?) async -> UInt64? {
        guard !busy else { return nil }
        generation &+= 1
        let ticket = generation
        ready = false
        guard let port else { notice = "Workflow runs are not connected on this runner yet."; return nil }
        do {
            let json = try await port.read(host, workspace)
            let pending = try await port.pending(host, workspace)
            guard ticket == generation else { return nil }
            let next = try WorkflowPresentation.decode(json, workspace: workspace)
            if presentation?.runs.contains(where: { old in
                !next.runs.contains(where: { $0.item == old.item && $0.run == old.run })
            }) == true { clearInspection() }
            retained = pending
            presentation = next
            ready = true; notice = ""
        } catch {
            guard ticket == generation else { return nil }
            notice = "Couldn’t refresh workflow runs. Reconnect or try Refresh."
        }
        return ticket == generation ? ticket : nil
    }
    public func perform(item: String, action: WorkflowAction, port: WorkflowPort?) async {
        guard ready, !busy, let port,
            let card = presentation?.runs.first(where: { $0.item == item }),
            card.actions.contains(action) else { return }
        let ticket = generation
        let display = selection
        busy = true; inspection = ""
        var failed = false
        do {
            let result = try await port.dispatch(host, workspace, item, action.json())
            if ticket == generation, display == selection,
                presentation?.runs.contains(where: { $0.item == item && $0.run == card.run }) == true {
                inspection = result
            }
        } catch {
            failed = true
        }
        guard ticket == generation else { return }
        busy = false; ready = false
        let current = await refresh(port)
        if failed, let current, current == generation { notice = "The action wasn’t confirmed. Check the refreshed run before trying again." }
    }
    public func retry(_ entry: RetainedWorkflow, port: WorkflowPort?) async {
        guard ready, !busy, let port else { return }
        let ticket = generation, display = selection
        busy = true; inspection = ""
        do {
            let result = try await port.retry(host, workspace, entry.id)
            if ticket == generation, display == selection,
                presentation?.runs.contains(where: { $0.item == entry.item && $0.run == entry.run }) == true {
                inspection = result
            }
        } catch { /* Keep the journal for the next explicit bounded retry. */ }
        guard ticket == generation else { return }
        busy = false; ready = false
        await refresh(port)
    }

}
