import Foundation
import Testing
@testable import AgentKit

struct WorkflowRunsTests {
    private func fixtures() throws -> [[String: Any]] {
        let source = URL(fileURLWithPath: #filePath)
        let root = source.deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        return try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("crates/client/src/testdata/workflow-presentation.json"))) as! [[String: Any]]
    }
    @Test @MainActor func goldenCopyAndOpaqueActions() async throws {
        for fixture in try fixtures() {
            let response = fixture["response"] as! [String: Any]
            let card = response["card"] as! [String: Any]
            let json = String(decoding: try JSONSerialization.data(withJSONObject: card), as: UTF8.self)
            let presentation = try WorkflowPresentation.decode(json, workspace: "workspace")
            #expect(!presentation.runs[0].label.isEmpty)
            let model = WorkflowRunsModel(host: "runner", workspace: "workspace")
            var forwarded: [String: String]?
            let port = WorkflowPort(id: "fixture-session", read: { host, workspace in
                #expect(host == "runner" && workspace == "workspace"); return json
            }, dispatch: { host, workspace, item, action in
                #expect(host == "runner" && workspace == "workspace" && item == presentation.runs[0].item)
                forwarded = try JSONSerialization.jsonObject(with: Data(action.utf8)) as? [String: String]
                return "retained evidence"
            })
            await model.refresh(port)
            for action in presentation.runs[0].actions {
                await model.perform(item: presentation.runs[0].item, action: action, port: port)
                #expect(forwarded?["guard"] == action.guardValue)
                #expect(forwarded?["label"] == action.label)
                #expect(forwarded?["kind"] == action.kind)
            }
            model.disconnect(); #expect(!model.ready)
            forwarded = nil
            if let action = presentation.runs[0].actions.first {
                await model.perform(item: presentation.runs[0].item, action: action, port: port)
                #expect(forwarded == nil)
            }
            await model.refresh(port); #expect(model.ready)
        }
    }
    @Test func rejectsWrongWorkspaceAndSchema() throws {
        #expect(throws: (any Error).self) { try WorkflowPresentation.decode("{\"schema\":1}", workspace: "other") }
    }
    @Test @MainActor func pendingPortDoesNotEnableActions() async {
        let model = WorkflowRunsModel(host: "runner", workspace: "workspace")
        await model.refresh(nil)
        #expect(!model.ready && model.notice.contains("not connected"))
    }
    @Test @MainActor func delayedDisplayLifecycle() async throws {
        let fixtures = try fixtures()
        let card = (fixtures[0]["response"] as! [String: Any])["card"]!
        try await workflowLifecycleChecks(json: String(decoding: try JSONSerialization.data(withJSONObject: card), as: UTF8.self))
    }

}
