import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// ⌘P finds workspaces and tasks (spec §4.8).
struct PaletteWorkspaceTests {
    private static let billing = PaletteWorkspace(
        host: "", id: "ws-bil", name: "Billing", repository: "overnight", hasOrchestrator: true)
    private static let lane = Worktree(
        id: "w-1", short: "w1", task: "fc-3-webhooks", branch: "fc-3-webhooks", repository: "overnight", host: "",
        path: "/tmp/w1", state: "active", terminals: [])

    /// A task key, and its title, find the task on a board already read,
    /// which opens in its workspace's task column.
    @Test("Typing a task key finds the task")
    func typingATaskKeyFindsTheTask() {
        let task = PaletteTask(
            host: "", workspace: "ws-bil", workspaceName: "Billing", id: "t-9", key: "bil-9",
            title: "Invoice PDF export", status: .inProgress)
        let found = PaletteIndex.matching("bil-9", in: [Self.lane], tasks: [task])
        let first = found.first
        #expect(first?.action == .openTask(host: "", workspace: "ws-bil", id: "t-9"))
        #expect(first?.title == "bil-9 Invoice PDF export")
        #expect(first?.detail == "Billing · In Progress")
        #expect(PaletteIndex.matching("invoice pdf", in: [Self.lane], tasks: [task]).first?.action
            == .openTask(host: "", workspace: "ws-bil", id: "t-9"))
    }

    /// Found by its name, whether or not any worktree matches; its
    /// orchestrator by "Billing Orchestrator". It used to be dropped when
    /// no worktree in it matched.
    @Test("A workspace with no matching worktree is still found")
    func aWorkspaceWithNoMatchingWorktreeIsStillFound() {
        let found = PaletteIndex.matching("billing", in: [Self.lane], workspaces: [Self.billing])
        #expect(found.contains { $0.action == .openWorkspace(host: "", id: "ws-bil") && $0.title == "Billing" })
        #expect(found.first { $0.kind == "workspace" }?.detail == "Workspace · overnight")
        let orchestrator = PaletteIndex.matching("billing orch", in: [Self.lane], workspaces: [Self.billing])
        #expect(orchestrator.first?.title == "Billing Orchestrator")
        // And creation is still last, under anything to go to.
        #expect(found.last?.action == .newWorktree("billing"))
    }
}
