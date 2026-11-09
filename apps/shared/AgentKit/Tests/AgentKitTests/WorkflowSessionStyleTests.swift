import Foundation
import Testing
@testable import AgentKit

/// Rust sets every label in the case it is told to: Apple's title case for Mac and iPhone. If this
/// adapter sent `web` nothing would change (the web page is title case too), and `material` would
/// only change the case, so each call it makes is held to `apple` here.
struct WorkflowSessionStyleTests {
    @Test @MainActor func everyCallToRustAsksForApplesCase() async throws {
        let source = URL(fileURLWithPath: #filePath)
        let root = (0..<6).reduce(source) { url, _ in url.deletingLastPathComponent() }
        let golden = try JSONSerialization.jsonObject(with: Data(contentsOf: root
            .appendingPathComponent("crates/client/src/testdata/workflow-presentation.json"))) as! [[String: Any]]
        let card = golden.compactMap { ($0["response"] as? [String: Any])?["card"] as? [String: Any] }
            .first { (($0["runs"] as? [[String: Any]])?.contains { !(($0["actions"] as? [Any]) ?? []).isEmpty }) ?? false }!
        let snapshot = String(decoding: try JSONSerialization.data(withJSONObject: ["host": "runner", "presentation": card]), as: UTF8.self)
        let journal = FileManager.default.temporaryDirectory.appendingPathComponent("workflow-style-\(UUID().uuidString)")
        defer { try? FileManager.default.removeItem(at: journal) }
        var calls: [(method: String, args: [String: Any])] = []
        let adapter = WorkflowSessionAdapter(root: journal, current: { _ in
            WorkflowSessionAdapter.Current(account: "account", host: "runner", connection: "connection", call: { method, args in
                calls.append((method, try JSONSerialization.jsonObject(with: Data(args.utf8)) as! [String: Any]))
                if method == "workflow.snapshot" { return snapshot }
                throw WorkflowDisplayError.pending // the reply to prepare does not matter: the request is what is held
            })
        })
        let port = try #require(adapter.port(host: "runner"))
        let workspace = card["workspace"] as! String
        let shown = try WirePresentation.decode(try await port.read("runner", workspace), workspace: workspace)
        let run = try #require(shown.runs.first { !$0.actions.isEmpty })
        _ = try? await port.dispatch("runner", workspace, run.item, run.actions[0].json())
        #expect(calls.map(\.method) == ["workflow.snapshot", "workflow.prepare_selection"])
        #expect(calls.map { $0.args["style"] as? String } == ["apple", "apple"])
    }
}
