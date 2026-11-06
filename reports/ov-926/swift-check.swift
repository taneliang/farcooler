import Foundation

// Standalone compiled model/controller harness; no UI launch. The production
// SwiftUI file is compiled in this executable too, so SwiftUI is typechecked.
@main struct WorkflowCheck {
    @MainActor static func main() async throws {
        let data = try Data(contentsOf: URL(fileURLWithPath: CommandLine.arguments[1]))
        let fixtures = try JSONSerialization.jsonObject(with: data) as! [[String: Any]]
        var count = 0
        for fixture in fixtures {
            let card = (fixture["response"] as! [String: Any])["card"] as! [String: Any]
            let json = String(decoding: try JSONSerialization.data(withJSONObject: card), as: UTF8.self)
            let presentation = try WorkflowPresentation.decode(json, workspace: "workspace")
            let run = presentation.runs[0]
            let model = WorkflowRunsModel(host: "runner", workspace: "workspace")
            var sent: String?
            let port = WorkflowPort(id: "fixture-session", read: { host, workspace in
                precondition(host == "runner" && workspace == "workspace"); return json
            }, dispatch: { host, workspace, item, action in
                precondition(host == "runner" && workspace == "workspace" && item == run.item)
                sent = action; return "retained evidence"
            })
            await model.refresh(port); precondition(model.ready)
            for action in run.actions {
                await model.perform(item: run.item, action: action, port: port)
                let forwarded = try JSONSerialization.jsonObject(with: Data(sent!.utf8)) as! [String: String]
                precondition(forwarded["guard"] == action.guardValue && forwarded["label"] == action.label && forwarded["kind"] == action.kind)
            }
            model.disconnect(); precondition(!model.ready)
            sent = nil
            await model.perform(item: run.item, action: run.actions[0], port: port); precondition(sent == nil)
            await model.refresh(port); precondition(model.ready)
            // A delayed read from a disconnected session must not re-enable actions.
            var resume: CheckedContinuation<String, Never>?
            let delayed = WorkflowPort(id: "old-session", read: { _, _ in
                await withCheckedContinuation { resume = $0 }
            }, dispatch: { _, _, _, _ in fatalError("must not submit") })
            let reading = Task { await model.refresh(delayed) }
            while resume == nil { await Task.yield() }
            model.disconnect(); resume!.resume(returning: json)
            await reading.value; precondition(!model.ready)
            count += 1; print("ok \(fixture["name"]!) shared copy/whole action/disconnect/reconnect")
        }
        let opaque = "{\"kind\":\"FutureAction\",\"guard\":\"exact\",\"label\":\"" + String(repeating: "Long shared label ", count: 100) + "\",\"extra\":\"preserved\"}"
        let action = try JSONDecoder().decode(WorkflowAction.self, from: Data(opaque.utf8))
        let roundtrip = try JSONSerialization.jsonObject(with: Data(action.json().utf8)) as! [String: String]
        precondition(roundtrip["extra"] == "preserved" && action.label.count > 1000)
        let pending = WorkflowRunsModel(host: "runner", workspace: "workspace")
        await pending.refresh(nil); precondition(!pending.ready && pending.notice.contains("not connected"))
        print("\(count) Rust golden cases and pending seam passed; SwiftUI typechecked, no app launch")
    }
}
