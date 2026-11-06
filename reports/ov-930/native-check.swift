import AppKit
import Foundation
import SwiftUI

// Private graphical evidence only. Production AgentKit adapter/model/view
// sources are compiled verbatim; HTTP is a test proxy to the real shared
// Session/socket fixture, not the application's RunnerCore/FFI implementation.
@MainActor final class NativeFixture: ObservableObject {
    @Published var port: WorkflowPort?
    @Published var status = "Connecting private fixture…"
    let root = URL(fileURLWithPath: ProcessInfo.processInfo.environment["OV930_NATIVE_ROOT"]!)
    let workspace: String
    let endpoint: URL
    private var hostID = ""
    private var generation = 1
    private var dropped = false
    private var calls: [[String: String]] = []
    private var adapter: WorkflowSessionAdapter?
    init() {
        let ready = try! JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("ready.json"))) as! [String: Any]
        workspace = ready["workspace"] as! String
        endpoint = URL(string: "http://127.0.0.1:\(ready["port"]!)/call")!
    }
    func call(_ method: String, _ raw: String) async throws -> String {
        var request = URLRequest(url: endpoint); request.httpMethod = "POST"
        request.httpBody = Data("{\"method\":\"\(method)\",\"args\":\(raw)}".utf8)
        let (data, _) = try await URLSession.shared.data(for: request)
        calls.append(["method": method, "args": raw, "connection": String(generation)])
        try JSONSerialization.data(withJSONObject: calls, options: .prettyPrinted).write(to: root.appendingPathComponent("calls.json"), options: .atomic)
        if method == "workflow.submit_action", !dropped { dropped = true; throw WorkflowDisplayError.pending }
        return String(decoding: data, as: UTF8.self)
    }
    func start() async {
        do {
            let raw = try await call("workflow.snapshot", "{\"workspace\":\"\(workspace)\"}")
            let snapshot = try JSONSerialization.jsonObject(with: Data(raw.utf8)) as! [String: Any]
            hostID = snapshot["host"] as! String
            restore()
            NSApp.activate(ignoringOtherApps: true)
        } catch { status = "Private fixture failed: \(error)" }
    }
    func restore() {
        generation += 1
        let next = WorkflowSessionAdapter(root: root.appendingPathComponent("journal")) { [weak self] _ in
            guard let self, !self.hostID.isEmpty else { return nil }
            return WorkflowSessionAdapter.Current(account: "private-fixture-account", host: self.hostID,
                connection: String(self.generation)) { [weak self] method, raw in
                    guard let self else { throw WorkflowDisplayError.pending }
                    return try await self.call(method, raw)
                }
        }
        adapter = next; port = next.port(host: "private-fixture-host")
        status = "Private FeatureA workspace · adapter generation \(generation)"
    }
}
@main struct NativeWorkflowCheck: App {
    @StateObject private var fixture = NativeFixture()
    var body: some Scene {
        WindowGroup("ov-930 Private Workflow Check") {
            VStack(alignment: .leading, spacing: 20) {
                Text("Private FeatureA Workspace").font(.title)
                Text(fixture.status)
                WorkflowRunsButton(host: "private-fixture-host", workspace: fixture.workspace)
                    .environment(\.workflowPort, fixture.port)
                Button("Restore Adapter From Private Journal") { fixture.restore() }
                Text("Development fixture only · no owner app or data").foregroundStyle(.secondary)
            }
            .padding(30).frame(minWidth: 900, minHeight: 900)
            .task { await fixture.start() }
        }
    }
}
