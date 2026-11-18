import SwiftUI

// Simulator-only host for the production run view and the Rust golden cards.
public struct RetainedWorkflow: Codable, Sendable, Identifiable {
    public let id: String
    let item: String
    let run: String
    let label: String
    var reinspect = false
}

@main struct WorkflowFixtureHost: App {
    private let json: String
    private let opened: Bool
    init() {
        let args = CommandLine.arguments
        let name = args.count > 1 ? args[1] : "approval"
        opened = args.contains("expanded")
        let url = Bundle.main.url(forResource: "workflow-presentation", withExtension: "json")!
        let fixtures = try! JSONSerialization.jsonObject(with: Data(contentsOf: url)) as! [[String: Any]]
        let fixture = fixtures.first { $0["name"] as? String == name }!
        let card = (fixture["response"] as! [String: Any])["card"] as! [String: Any]
        let run = (card["runs"] as! [[String: Any]])[0]
        UserDefaults.standard.set(run["item"] as! String, forKey: "workflow.run.fixture.workspace")
        json = String(data: try! JSONSerialization.data(withJSONObject: card), encoding: .utf8)!
    }
    var body: some Scene {
        WindowGroup {
            WorkflowRunsView(host: "fixture", workspace: "workspace", detailsOpen: opened)
                .environment(\.workflowPort, WorkflowPort(id: "fixture", read: { _, _ in json }, dispatch: { _, _, _, _ in "" }))
        }
    }
}
