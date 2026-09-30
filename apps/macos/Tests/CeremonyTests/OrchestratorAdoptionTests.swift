import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// Use as Orchestrator and Stop Being Orchestrator: what's offered, what
/// steps down first, and a refusal in words (ov-63).
@MainActor
struct OrchestratorAdoptionTests {
    private static let repo = "0198f2c0-0000-7000-8000-0000000000aa"
    private static let main = "0198f2c0-0000-7000-8000-0000000000cc"
    private static let billing = "0198f2c0-0000-7000-8000-0000000000dd"

    private static func terminal(
        _ id: String, preset: String, state: String = "running", workspace: String? = main, role: String? = nil
    ) -> Terminal {
        var t = Terminal(id: id, short: id, title: id, preset: preset, state: state, epoch: 0)
        t.workspace = workspace
        t.role = role
        return t
    }

    /// The owner's Main: claude run by hand in a shell, sleepnomore beside
    /// it, a stopped shell, and Billing's orchestrator.
    private static func fleet(seated: String? = nil, extra: [Terminal] = []) -> Fleet {
        var checkout = Worktree(
            id: "checkout", short: "co", task: "main", branch: "main", repository: "overnight", host: "",
            path: "/tmp/co", state: "active",
            terminals: [
                terminal("claude", preset: "claude"), terminal("sleep", preset: "zsh"),
                terminal("gone", preset: "zsh", state: "exited"),
                terminal("bill", preset: "claude", workspace: billing, role: "orchestrator"),
            ] + extra,
            repositoryID: repo, workspace: main)
        checkout.is_main_checkout = true
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [checkout], branchPrefix: nil)
        fleet.runnerWorkspaces[""] = [
            WorkspaceSummary(id: main, name: "Main", taskPrefix: "fc", isMain: true, ordinal: 0, repository: repo, orchestrator: seated),
            WorkspaceSummary(id: billing, name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: repo, orchestrator: "bill"),
        ]
        return fleet
    }

    @Test("Running terminals of a listed workspace are offered, orchestrators step down")
    func whatIsOffered() {
        let fleet = Self.fleet()
        let w = fleet.worktrees[0]
        let t = w.terminals
        #expect(OrchestratorAdoption.offer(for: t[0], in: w, host: "", fleet: fleet) == .use)
        #expect(OrchestratorAdoption.offer(for: t[1], in: w, host: "", fleet: fleet) == .use)
        #expect(OrchestratorAdoption.offer(for: t[2], in: w, host: "", fleet: fleet) == nil, "an exited terminal was offered")
        #expect(OrchestratorAdoption.offer(for: t[3], in: w, host: "", fleet: fleet) == .stepDown)
        // No workspace, or a runner without workspaces: nothing to run.
        #expect(OrchestratorAdoption.offer(for: Self.terminal("x", preset: "zsh", workspace: nil), in: w, host: "", fleet: fleet) == nil)
        var old = fleet
        old.runnerWorkspaces[""] = nil
        #expect(OrchestratorAdoption.offer(for: t[0], in: w, host: "", fleet: old) == nil)

        // A task's agent works its task, and a task worktree isn't where
        // orchestrators run: neither is offered.
        var agent = Self.terminal("a", preset: "claude")
        agent.taskId = "t-1"
        #expect(OrchestratorAdoption.offer(for: agent, in: w, host: "", fleet: fleet) == nil, "a task's agent was offered")
        var lane = w
        lane.is_main_checkout = false
        #expect(OrchestratorAdoption.offer(for: t[0], in: lane, host: "", fleet: fleet) == nil, "a task worktree's terminal was offered")

        // The empty state lists Main's own running terminals, by the
        // runner's order, and not Billing's orchestrator.
        let main = fleet.runnerWorkspaces[""]![0]
        #expect(OrchestratorAdoption.candidates(for: main, host: "", in: fleet).map(\.terminal.id) == ["claude", "sleep"])
    }

    @Test("An orchestrator already seated is named, to step down first")
    func whatIsReplaced() {
        var fleet = Self.fleet(seated: "claude")
        fleet.worktrees[0].terminals[0].role = "orchestrator"
        let main = fleet.runnerWorkspaces[""]![0]
        let sleep = BoardPane(terminal: fleet.worktrees[0].terminals[1], worktree: fleet.worktrees[0])
        #expect(OrchestratorAdoption.replacing(sleep, in: main, host: "", fleet: fleet)?.terminal.id == "claude")
        let claude = BoardPane(terminal: fleet.worktrees[0].terminals[0], worktree: fleet.worktrees[0])
        #expect(OrchestratorAdoption.replacing(claude, in: main, host: "", fleet: fleet) == nil)
        let unseated = Self.fleet()
        #expect(OrchestratorAdoption.replacing(sleep, in: unseated.runnerWorkspaces[""]![0], host: "", fleet: unseated) == nil)

        // A lost seat isn't replaced, or named as running: the runner
        // vacates it itself.
        var lost = fleet
        lost.worktrees[0].terminals[0].state = "lost"
        #expect(OrchestratorAdoption.replacing(sleep, in: main, host: "", fleet: lost) == nil)

        // It steps down to what it would have been made as.
        #expect(OrchestratorAdoption.steppedDown(fleet.worktrees[0].terminals[0]) == "agent")
        #expect(OrchestratorAdoption.steppedDown(fleet.worktrees[0].terminals[1]) == "shell")
        var handoff = fleet.worktrees[0].terminals[0]
        handoff.taskId = "t-1"
        #expect(OrchestratorAdoption.steppedDown(handoff) == "shell", "a handoff's claude would join its task's column")
    }

    @Test("A refusal is a sentence")
    func refusals() {
        #expect(
            DaemonClient.setRoleArguments(terminal: "5a7573bd", role: "orchestrator")
                == ["terminal", "set-role", "5a7573bd", "orchestrator", "--json"])
        let taken = "error: that workspace already has an orchestrator\ncode: invalid-argument\nwhat: orchestrator_taken"
        #expect(OrchestratorAdoption.refusal(taken, terminal: "claude", workspace: "Main") == "Main has another orchestrator now. Try again to replace it.")
        let loose = "error: x\ncode: invalid-argument\nwhat: workspace"
        #expect(OrchestratorAdoption.refusal(loose, terminal: "claude", workspace: "Main") == "claude isn’t in a workspace, so it can’t be an orchestrator.")
        #expect(OrchestratorAdoption.refusal("code: not-found", terminal: "claude", workspace: "Main") == "claude isn’t on this runner anymore.")
        #expect(OrchestratorAdoption.refusal("code: resource-conflict", terminal: "claude", workspace: "Main") == "Main changed while you were choosing. Try again.")
        #expect(OrchestratorAdoption.refusal(nil, terminal: "claude", workspace: "Main").hasPrefix("Couldn’t change what claude is."))
        for message in [taken, loose, "code: internal"] {
            let said = OrchestratorAdoption.refusal(message, terminal: "claude", workspace: "Main")
            #expect(!said.contains("code:") && !said.contains("orchestrator_taken"), "\(said)")
        }
    }
}
