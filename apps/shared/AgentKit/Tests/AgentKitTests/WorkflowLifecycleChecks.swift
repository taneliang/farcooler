import Foundation
#if !WORKFLOW_STANDALONE
@testable import AgentKit
#endif

private enum WorkflowLifecycleFailure: Error { case staleDisplay }
private func workflowRequire(_ condition: Bool) throws {
    if !condition { throw WorkflowLifecycleFailure.staleDisplay }
}

// Deterministic continuations hold the action/read at the reviewed race boundary.
// Shared with the bounded standalone harness so execution and XCTest use the
// same regressions without building the full application.
@MainActor func workflowLifecycleChecks(json: String) async throws {
    let card = try WorkflowPresentation.decode(json, workspace: "workspace").runs[0]
    for returnToOriginal in [false, true] {
        let model = WorkflowRunsModel(host: "runner", workspace: "workspace")
        var reply: CheckedContinuation<String, Never>?
        var reads = 0
        let port = WorkflowPort(id: "old", read: { _, _ in reads += 1; return json }, dispatch: { _, _, _, _ in
            await withCheckedContinuation { reply = $0 }
        })
        await model.refresh(port)
        let sending = Task { await model.perform(item: card.item, action: card.actions[0], port: port) }
        while reply == nil { await Task.yield() }
        model.clearInspection() // All Runs / select another item
        if returnToOriginal { model.clearInspection() }
        reply!.resume(returning: "old evidence"); await sending.value
        try workflowRequire(model.inspection.isEmpty && !model.busy && model.ready && reads == 2)
    }
    for reconnect in [false, true] {
        let model = WorkflowRunsModel(host: "runner", workspace: "workspace")
        var reply: CheckedContinuation<String, Never>?
        let old = WorkflowPort(id: "old", read: { _, _ in json }, dispatch: { _, _, _, _ in
            await withCheckedContinuation { reply = $0 }
        })
        await model.refresh(old)
        let sending = Task { await model.perform(item: card.item, action: card.actions[0], port: old) }
        while reply == nil { await Task.yield() }
        model.disconnect()
        if reconnect { await model.refresh(old) }
        reply!.resume(returning: "old evidence"); await sending.value
        try workflowRequire(model.inspection.isEmpty && !model.busy && model.ready == reconnect)
    }
    for duringAction in [false, true] {
        let model = WorkflowRunsModel(host: "runner", workspace: "workspace")
        var object = try JSONSerialization.jsonObject(with: Data(json.utf8)) as! [String: Any]
        var runs = object["runs"] as! [[String: Any]]
        runs[0]["run"] = "replacement"; object["runs"] = runs
        let replacement = String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
        var current = json
        let port = WorkflowPort(id: "old", read: { _, _ in current }, dispatch: { _, _, _, _ in
            if duringAction { current = replacement }; return "old evidence"
        })
        await model.refresh(port); await model.perform(item: card.item, action: card.actions[0], port: port)
        if !duringAction {
            try workflowRequire(model.inspection == "old evidence"); current = replacement; await model.refresh(port)
        }
        try workflowRequire(model.inspection.isEmpty)
    }
    for reconnect in [false, true] {
        let model = WorkflowRunsModel(host: "runner", workspace: "workspace")
        var reply: CheckedContinuation<String, Never>?
        var reads = 0
        let old = WorkflowPort(id: "old", read: { _, _ in
            reads += 1
            if reads == 1 { return json }
            return await withCheckedContinuation { reply = $0 }
        }, dispatch: { _, _, _, _ in throw WorkflowDisplayError.pending })
        await model.refresh(old)
        let sending = Task { await model.perform(item: card.item, action: card.actions[0], port: old) }
        while reply == nil { await Task.yield() }
        model.disconnect()
        if reconnect {
            await model.refresh(WorkflowPort(id: "new", read: { _, _ in json }, dispatch: { _, _, _, _ in
                preconditionFailure("must not replay")
            }))
        }
        let notice = model.notice
        reply!.resume(returning: json); await sending.value
        try workflowRequire(model.notice == notice && model.ready == reconnect && model.inspection.isEmpty)
    }
}
