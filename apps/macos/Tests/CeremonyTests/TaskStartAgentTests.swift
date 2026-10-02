import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// ov-81 P3: a task with no worktree can start an agent, or go onto an
/// existing worktree, from its own screen.
@MainActor
struct TaskStartAgentTests {
    private func client(_ runner: StartTaskTests.Runner) async -> DaemonClient {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in runner.answer(args) }
        client.copyToClipboard = { _ in }
        await client.refresh()
        for _ in 0..<100 where client.daemonBuild == nil {
            try? await Task.sleep(for: .milliseconds(10))
        }
        return client
    }

    private var row: TaskRow {
        var row = TaskRow(
            id: "t-9", key: "bil-9", title: "Invoice PDF export", status: .backlog, statusSince: .now)
        row.intent = "Export an invoice as a PDF."
        return row
    }

    @Test("Start Agent makes a worktree, starts the agent on the task, and puts the task on that lane")
    func startsAndLinks() async {
        let runner = StartTaskTests.Runner(capabilities: ["workspaces", "terminals", "launch_prompt"])
        let client = await client(runner)
        let outcome = await client.startAgent(
            for: row, repository: "repo", workspace: nil, agent: "claude")
        #expect(outcome == .started(worktree: "w-new", terminal: "t-new"))
        #expect(runner.made("worktree") != nil)
        let prompt = runner.made("terminal")?.first { $0.hasPrefix("--prompt=") }
        #expect(prompt?.contains("bil-9: Invoice PDF export") == true)
        #expect(prompt?.contains("Export an invoice as a PDF.") == true)
        let link = runner.calls.first { $0.starts(with: ["task", "set"]) }
        #expect(link == ["task", "set", "bil-9", "--worktree", "w-new", "--repo", "repo", "--json"])
    }

    @Test("A start whose link fails says so and names the worktree it made")
    func linkFailureIsSaid() async {
        let runner = StartTaskTests.Runner(capabilities: ["workspaces", "terminals", "launch_prompt"])
        runner.linkFails = true
        let client = await client(runner)
        let outcome = await client.startAgent(
            for: row, repository: "repo", workspace: nil, agent: "claude")
        guard case .failed(let sentence, let made) = outcome else {
            Issue.record("expected a failure, got \(outcome)")
            return
        }
        #expect(sentence.contains("attach"))
        #expect(made?.id == "w-new")
    }

    @Test("Attachable worktrees are this repository's free linked ones")
    func attachable() {
        func worktree(_ id: String, repo: String = "r1", main: Bool = false, state: String = "active") -> Worktree {
            var w = Worktree(
                id: id, short: id, task: id, branch: id, repository: "Billing", host: "",
                path: "/tmp/\(id)", state: state, terminals: [])
            w.repositoryID = repo
            w.is_main_checkout = main
            return w
        }
        let used = { () -> TaskRow in
            var r = TaskRow(id: "t-1", key: "bil-1", title: "x", status: .inProgress, statusSince: .now)
            r.worktreeID = "w-used"
            return r
        }()
        let found = TaskColumnModel.attachable(
            [
                worktree("w-free"), worktree("w-used"), worktree("w-main", main: true),
                worktree("w-other", repo: "r2"), worktree("w-hidden", state: "hidden"),
                worktree("w-gone", state: "worktree_missing"),
            ], host: "", repository: "r1", taken: [used])
        #expect(found.map(\.id) == ["w-free"])
    }
}
