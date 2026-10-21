import Foundation
import Testing

@testable import AgentKit

/// All Agents (ov-676): a list that mixes repositories names each group by its
/// repository, and an idle orchestrator says what it's for in full.
struct AllAgentsNamingTests {
    static func workspace(_ id: String, _ name: String, repository: String, main: Bool) -> WorkspaceSummary {
        WorkspaceSummary(
            id: id, name: name, taskPrefix: "x", isMain: main, ordinal: main ? 0 : 1, repository: repository)
    }

    @Test("Two repositories' main checkouts are headed by two different names")
    func mainCheckoutsAreToldApartByRepository() {
        let cosmo = Self.workspace("w1", "Main", repository: "r1", main: true)
        let nova = Self.workspace("w2", "Main", repository: "r2", main: true)
        let a = cosmo.listedName(repositoryName: "Cosmo")
        let b = nova.listedName(repositoryName: "Nova")
        #expect(a == "Cosmo")
        #expect(b == "Nova")
        #expect(a != b)
    }

    @Test("Another workspace says its repository, then itself; a name not yet known leaves the workspace's own")
    func otherWorkspacesNameBoth() {
        let billing = Self.workspace("w3", "Billing", repository: "r1", main: false)
        #expect(billing.listedName(repositoryName: "Cosmo") == "Cosmo · Billing")
        #expect(billing.listedName(repositoryName: nil) == "Billing")
        let main = Self.workspace("w1", "Main", repository: "r1", main: true)
        #expect(main.listedName(repositoryName: nil) == "Main")
    }

    @Test("An idle orchestrator's preview is its whole title, not the runner's 18-character headline")
    func idleOrchestratorSaysWhatItIsFor() {
        func orchestrator(_ activity: String, line: String) -> Terminal {
            RunningAgentsTests.terminal(
                "orch", title: "Minimize CI costs across every runner", activity: activity, workspace: "ws",
                role: "orchestrator", line: line)
        }
        func preview(_ terminal: Terminal) -> String {
            let worktree = RunningAgentsTests.worktree("w", workspace: "ws", [terminal])
            let pane = AgentPane(terminal, in: worktree, workspace: "ws")
            return RunningAgents.orchestrator(pane, runner: "R").activity
        }
        #expect(preview(orchestrator("idle", line: "Minimize CI costs…")) == "Minimize CI costs across every runner")
        #expect(preview(orchestrator("done", line: "Minimize CI costs…")) == "Minimize CI costs across every runner")
        // Working and blocked have something better to say: keep the runner's line.
        #expect(preview(orchestrator("working", line: "Running cargo test")) == "Running cargo test")
        #expect(preview(orchestrator("blocked", line: "Push to main?")) == "Push to main?")
    }
}
