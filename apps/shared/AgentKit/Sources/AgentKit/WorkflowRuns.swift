import Foundation
import Combine

// Display-only schema 2. The runs are the Rust core's presentation, generated
// as `WireTypes.swift` (ov-1035); these names are the apps' own for them.
public typealias WorkflowAction = WireAction
public typealias WorkflowStage = WireStageCard
public typealias WorkflowGate = WireGateCard
public typealias WorkflowReason = WireReasonCard
public typealias WorkflowEvidence = WireEvidence
public typealias WorkflowForm = WireActionForm
public typealias WorkflowRework = WireReworkDecision
public typealias WorkflowRun = WireRunCard
public typealias WorkflowPresentation = WirePresentation

extension WireAction {
    public var guardValue: String { `guard` }
    public func selectionJSON(reason: String?) throws -> String {
        guard let reason else { return try json() }
        struct Selection: Encodable {
            var action: WireAction
            var reason: String
        }
        let encoder = JSONEncoder()
        encoder.outputFormatting = .sortedKeys
        return String(decoding: try encoder.encode(Selection(action: self, reason: reason)), as: UTF8.self)
    }
    public func json() throws -> String { String(decoding: try JSONEncoder().encode(self), as: UTF8.self) }
}

extension WireRunCard: Identifiable {
    public var id: String { item }
}

extension WirePresentation {
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
    public func perform(item: String, action: WorkflowAction, port: WorkflowPort?, reason: String? = nil) async {
        guard ready, !busy, let port,
            let card = presentation?.runs.first(where: { $0.item == item }),
            card.actions.contains(action) else { return }
        let ticket = generation
        let display = selection
        busy = true; inspection = ""
        var failed = false
        do {
            let result = try await port.dispatch(host, workspace, item, action.selectionJSON(reason: reason))
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
