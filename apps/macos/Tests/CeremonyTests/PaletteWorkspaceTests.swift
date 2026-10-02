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
    /// which opens drilled into, in its workspace.
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

    /// Worktrees rank above the terminals found only by where they are
    /// (ov-86): typing a worktree's name lists the worktrees it could mean
    /// before the panes inside any of them, which the worktree's own name
    /// would otherwise tie with and interleave.
    @Test("A worktree outranks the terminals found by its name")
    func worktreesOutrankTheirTerminals() {
        func lane(_ id: String, _ name: String) -> Worktree {
            Worktree(
                id: id, short: id, task: name, branch: name, repository: "overnight", host: "", path: "/tmp/\(id)",
                state: "active",
                terminals: ["zsh", "claude"].map {
                    Terminal(id: "\(id)-\($0)", short: "\(id)-\($0)", title: $0, preset: $0, state: "running", epoch: 0)
                })
        }
        let found = PaletteIndex.matching("api", in: [lane("w1", "api-auth"), lane("w2", "api-billing")])
        let kinds = found.filter { $0.kind != "action" }.map(\.kind)
        #expect(kinds.prefix(2) == ["worktree", "worktree"], "\(found.map(\.title))")
    }

    /// A worktree is found by the key of the task it's for: "bil-9" goes to
    /// the task, and to its worktree too.
    @Test("A worktree is found by its task's key")
    func aWorktreeIsFoundByItsTasksKey() {
        var lane = Self.lane
        lane.openTasks = [NeedsYouTask(id: "t-9", key: "bil-9", title: "Invoice PDF export", status: "in_progress")]
        let found = PaletteIndex.matching("bil-9", in: [lane])
        let row = found.first { $0.action == .openWorktree("w-1") }
        #expect(row != nil)
        #expect(row?.detail.contains("bil-9") == true, "\(row?.detail ?? "")")
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

    /// New Workspace… is offered, named what was typed, below everything
    /// to go to and never on a runner without workspaces.
    @Test("Typing a name offers New Workspace, below everything to go to")
    func typingANameOffersNewWorkspace() {
        let found = PaletteIndex.matching("billing", in: [Self.lane], workspaces: [Self.billing], offersNewWorkspace: true)
        let at = found.firstIndex { $0.action == .newWorkspace("billing") }
        #expect(at != nil)
        #expect(found.first { $0.action == .newWorkspace("billing") }?.title == "New Workspace “billing”…")
        if let at { #expect(!found[at...].contains { $0.kind != "action" }, "creation above something to go to") }
        #expect(!PaletteIndex.matching("billing", in: [Self.lane]).contains { $0.id == "new-workspace" })
    }

    /// Both creation items open something to fill in, so both say so with an
    /// ellipsis; one of them used to leave it off.
    @Test("Both creation items end in an ellipsis")
    func creationItemsEndInAnEllipsis() {
        let found = PaletteIndex.matching("billing", in: [Self.lane], workspaces: [Self.billing], offersNewWorkspace: true)
        #expect(found.first { $0.id == "new-task" }?.title == "New Worktree “billing”…")
        #expect(found.first { $0.id == "new-workspace" }?.title == "New Workspace “billing”…")
    }

    /// Left empty, the prefix comes from the name, lowercase letters only,
    /// and never one the runner's workspaces use already: the runner
    /// requires one, and refuses a taken one.
    @Test("A new workspace's prefix is derived from its name")
    func aNewWorkspacesPrefixIsDerivedFromItsName() {
        #expect(WorkspacePrefix.derive(name: "Billing", taken: []) == "bil")
        #expect(WorkspacePrefix.derive(name: "Billing", taken: ["bil"]) == "bill")
        #expect(WorkspacePrefix.derive(name: "Relay rewrite", taken: ["rel", "rela", "relay"]) == "relayr")
        #expect(WorkspacePrefix.derive(name: "Ops", taken: ["ops"]) == "ops2")
        #expect(WorkspacePrefix.derive(name: "2026 Q4!", taken: []) == "q")
        #expect(WorkspacePrefix.derive(name: "", taken: []) == "ws")
        #expect(WorkspacePrefix.derive(name: "Élan", taken: []) == "lan")
        for name in ["Billing", "Ops", "", "A really long workspace name"] {
            let prefix = WorkspacePrefix.derive(name: name, taken: [])
            #expect(prefix.first?.isLetter == true && prefix.count <= 8, "\(prefix)")
        }
    }
}
