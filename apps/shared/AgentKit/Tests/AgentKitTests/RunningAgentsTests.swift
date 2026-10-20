import Foundation
import Testing

@testable import AgentKit

/// The agents at work (ov-619, ov-617): Far Cooler's panes and Claude's
/// subagents read as one list, scoped to a workspace for the Agents card and
/// across runners for All Agents. The fleet decides how many (the larger of a
/// pane's count and its names; a reconnect's fresh fleet is the whole of
/// keeping it right); a pane's conversation adds detail, joined on call ids,
/// and fills in the unnamed rest.
struct RunningAgentsTests {
    static let runner = "RUNNER-A"
    static let main = "ws-main"
    static let billing = "ws-billing"

    static func terminal(
        _ id: String, title: String = "claude", activity: String? = "working", workspace: String? = nil,
        role: String? = "agent", state: String = "running", line: String? = nil, turn: Double? = nil,
        subagents: [String]? = nil, running: Int? = nil, ids: [String]? = nil
    ) -> Terminal {
        Terminal(
            id: id, short: id, title: title, preset: "claude", state: state, activity: activity,
            turnStartedAt: turn, line: line, subagents: subagents, subagentsRunning: running, subagentIds: ids, epoch: 1,
            paneMode: "terminal", workspace: workspace, role: role)
    }

    static func worktree(_ id: String, workspace: String?, _ terminals: [Terminal]) -> Worktree {
        Worktree(
            id: id, short: id, repository: "repo", task: id, branch: "b/\(id)", state: "ready", terminals: terminals,
            workspace: workspace)
    }

    static func entry(
        _ id: String, _ title: String, asked: String, running: Bool = true, started: Int64? = nil, call: String? = nil
    ) -> AgentTray.Entry {
        AgentTray.Entry(
            id: id, isMain: false, title: title, description: asked, action: "Read x", running: running,
            startedMs: started, agentId: "agent-\(id)", toolUseId: call)
    }

    /// The orchestrator alone, with `subagents` named under the call ids
    /// `ids` and `running` running.
    static func orchestrating(_ subagents: [String], running: Int?, ids: [String]? = nil) -> Fleet {
        Fleet(
            runtimeHealthy: true, livePanes: 1,
            worktrees: [
                worktree("checkout", workspace: main, [
                    terminal("orch", workspace: main, role: "orchestrator", subagents: subagents, running: running, ids: ids)
                ])
            ],
            workspaces: [])
    }

    /// Main's orchestrator with two subagents (named, as the runner names
    /// them, by what they were asked), a lane agent with one, a shell, an
    /// exited agent; Billing's own agent; an unclaimed worktree's agent.
    static var fleet: Fleet {
        Fleet(
            runtimeHealthy: true, livePanes: 6,
            worktrees: [
                worktree("checkout", workspace: main, [
                    terminal(
                        "orch", workspace: main, role: "orchestrator", subagents: ["Survey the repo", "Review ov-616"],
                        running: 2, ids: ["toolu_s", "toolu_r"]),
                    terminal("shell", activity: nil, workspace: main, role: "shell"),
                ]),
                worktree("lane", workspace: main, [
                    terminal(
                        "builder", title: "Build ov-619", workspace: main, line: "Edit RunningAgents.swift",
                        turn: 1_000, subagents: ["Survey the repo"]),
                    terminal("gone", workspace: main, state: "exited"),
                ]),
                worktree("bill", workspace: billing, [terminal("biller", activity: "blocked", workspace: billing)]),
                worktree("scratch", workspace: nil, [terminal("loose", activity: "idle")]),
            ],
            workspaces: [])
    }

    @Test("A workspace lists its orchestrator's subagents, then each agent pane with its own; never the orchestrator, a shell or an exited pane")
    func workspaceScope() {
        let agents = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: Self.fleet)
        #expect(agents.agents.map(\.name) == ["Subagent", "Subagent", "Build ov-619", "Subagent"], "an unknown type is numbered")
        #expect(agents.agents.map(\.activity) == ["Survey the repo", "Review ov-616", "Edit RunningAgents.swift", "Survey the repo"])
        #expect(agents.agents.map(\.kind) == [.subagent, .subagent, .pane, .subagent])
        #expect(agents.count == 4)
        #expect(agents.summary == "4 agents")
        #expect(agents.dots.count == RunningAgents.dotLimit)
        let builder = agents.agents[2]
        #expect(builder.activity == "Edit RunningAgents.swift")
        #expect(builder.startedMs == 1_000)
        #expect(builder.worktree == "lane" && builder.terminal == "builder")
        // A subagent known only by name opens its parent's pane.
        #expect(agents.agents[0].terminal == "orch" && !agents.agents[0].opensOwnConversation)
        #expect(Set(agents.agents.map(\.id)).count == agents.agents.count, "every row has its own id")
    }

    @Test("With none at work the card has nothing to count, and collapses")
    func noneRunning() {
        let quiet = Fleet(
            runtimeHealthy: true, livePanes: 1,
            worktrees: [Self.worktree("checkout", workspace: Self.main, [Self.terminal("orch", workspace: Self.main, role: "orchestrator")])],
            workspaces: [])
        let agents = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: quiet)
        #expect(agents.count == 0)
        #expect(agents.dots.isEmpty)
        #expect(RunningAgents.summary(1) == "1 agent")
    }

    @Test("A conversation the phone holds adds each named subagent's type, newest work and start, and opens it")
    func conversationAddsDetail() {
        let rows = [
            AgentTray.Entry(id: AgentTray.mainID, isMain: true, title: "main", running: true),
            Self.entry("sub:a1", "General purpose", asked: "Review ov-616", started: 5_000, call: "toolu_r"),
        ]
        let agents = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: Self.fleet) { $0 == "orch" ? rows : nil }
        let review = agents.agents[1]
        #expect(review.name == "General purpose")
        #expect(review.activity == "Review ov-616")
        #expect(review.startedMs == 5_000)
        #expect(review.opensOwnConversation)
        #expect(review.row == "sub:a1" && review.agentId == "agent-sub:a1")
        #expect(agents.count == 4)
    }

    @Test("The fleet decides the count: a row stuck at running adds nothing, and an empty conversation hides nothing")
    func theFleetDecides() {
        // The conversation still says an agent the runner has finished runs.
        let stuck = [
            Self.entry("sub:a1", "General purpose", asked: "Review ov-616", call: "toolu_r"),
            Self.entry("sub:old", "General purpose", asked: "An agent that ended", call: "toolu_old"),
        ]
        let withStuck = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: Self.fleet) { $0 == "orch" ? stuck : nil }
        #expect(withStuck.count == 4, "an ended agent was counted")
        #expect(!withStuck.agents.contains { $0.activity == "An agent that ended" })

        // A conversation that hasn't loaded its subagents yet.
        let empty = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: Self.fleet) { _ in [] }
        #expect(empty.count == 4, "a running agent was hidden")
        #expect(empty.agents.map(\.activity).prefix(2) == ["Survey the repo", "Review ov-616"])
    }

    @Test("A lane pane's own conversation adds detail to its subagents too")
    func lanePaneRows() {
        let rows = [Self.entry("sub:b1", "Explore", asked: "Survey the repo for the card")]
        let agents = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: Self.fleet) { $0 == "builder" ? rows : nil }
        let sub = agents.agents[3]
        #expect(sub.terminal == "builder")
        #expect(sub.name == "Explore" && sub.opensOwnConversation)
    }

    @Test("Two subagents of one type are told apart by number")
    func duplicatesAreNumbered() {
        let rows = [
            Self.entry("sub:a1", "General purpose", asked: "Survey the repo", call: "toolu_s"),
            Self.entry("sub:a2", "General purpose", asked: "Review ov-616", call: "toolu_r"),
        ]
        let agents = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: Self.fleet) { $0 == "orch" ? rows : nil }
        #expect(agents.agents.prefix(2).map(\.name) == ["General purpose", "General purpose 2"])
        #expect(agents.agents.prefix(2).map(\.activity) == ["Survey the repo", "Review ov-616"])
    }

    @Test("A reconnect's fresh fleet is the whole of the count: the same fleet twice counts the same, and a new one recounts")
    func recountsFromTheFleet() {
        var fleet = Self.fleet
        let before = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: fleet)
        #expect(RunningAgents.workspace(Self.main, runner: Self.runner, fleet: fleet) == before)
        fleet.worktrees[1].terminals[0].state = "exited"
        fleet.worktrees[0].terminals[0].subagents = []
        fleet.worktrees[0].terminals[0].subagentsRunning = 0
        #expect(RunningAgents.workspace(Self.main, runner: Self.runner, fleet: fleet).count == 0)
    }

    @Test("All Agents: each workspace's orchestrator leads its own, workspaces in the fleet's order, the unclaimed last")
    func everywhere() {
        let agents = RunningAgents.runner(Self.runner, fleet: Self.fleet)
        #expect(agents.agents.map(\.name) == [
            RunningAgents.orchestratorName, "Subagent", "Subagent", "Build ov-619", "Subagent", "claude", "claude",
        ])
        #expect(agents.byWorkspace.map(\.workspace) == [Self.main, Self.billing, nil])
        #expect(agents.agents[0].kind == .orchestrator)
        #expect(agents.count == agents.agents.count - 1)
    }

    @Test("Agents a turn left running in the background are counted after it ends: no names, a count of 3, counts 3")
    func backgroundAgentsOutliveTheirTurn() {
        let agents = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: Self.orchestrating([], running: 3))
        #expect(agents.count == 3, "the card collapsed while three agents ran")
        #expect(agents.summary == "3 agents")
        #expect(agents.dots.count == 3)
        #expect(agents.agents.map(\.name) == ["3 more agents"])
        #expect(agents.agents.first?.terminal == "orch" && agents.agents.first?.opensOwnConversation == false)
        #expect(RunningAgents.more(1) == "1 more agent")
    }

    @Test("A held conversation fills the unnamed rest from its newest running rows, and the count still caps it")
    func aHeldConversationFillsTheRest() {
        let rows = [
            Self.entry("sub:old", "General purpose", asked: "Stuck at running", call: "toolu_0"),
            Self.entry("sub:b1", "Explore", asked: "Survey the repo", call: "toolu_1"),
            Self.entry("sub:b2", "General purpose", asked: "Review ov-616", call: "toolu_2"),
            Self.entry("sub:done", "General purpose", asked: "Finished", running: false, call: "toolu_3"),
        ]
        let fleet = Self.orchestrating([], running: 2)
        let agents = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: fleet) { $0 == "orch" ? rows : nil }
        #expect(agents.count == 2)
        #expect(agents.agents.map(\.name) == ["Explore", "General purpose"])
        #expect(agents.agents.map(\.activity) == ["Survey the repo", "Review ov-616"])
        #expect(agents.agents.allSatisfy { $0.opensOwnConversation })

        // Fewer rows than the count: the rest is still said.
        let short = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: Self.orchestrating([], running: 4)) {
            $0 == "orch" ? [rows[1]] : nil
        }
        #expect(short.count == 4)
        #expect(short.agents.map(\.name) == ["Explore", "3 more agents"])
    }

    @Test("A name the runner cleaned (a path narrowed) still finds its row, by the call id behind it")
    func joinsOnTheCallId() {
        let fleet = Self.orchestrating(["Read \u{2026}/daemon/src/watch.rs closely"], running: 1, ids: ["toolu_x"])
        let rows = [
            Self.entry("sub:y", "Explore", asked: "Something else", call: "toolu_y"),
            Self.entry("sub:x", "General purpose", asked: "Read /Users/someone/Dev/acme/crates/daemon/src/watch.rs closely", call: "toolu_x"),
        ]
        let agents = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: fleet) { $0 == "orch" ? rows : nil }
        #expect(agents.count == 1)
        #expect(agents.agents.first?.row == "sub:x", "the row wasn't found by its call id")
        #expect(agents.agents.first?.name == "General purpose")
    }

    @Test("From a runner that sends no ids or count, names take rows by order, and the names are the count")
    func anOlderRunner() {
        let fleet = Self.orchestrating(["Survey the repo"], running: nil)
        let rows = [Self.entry("sub:a", "Explore", asked: "Survey the repo for the card")]
        let agents = RunningAgents.workspace(Self.main, runner: Self.runner, fleet: fleet) { $0 == "orch" ? rows : nil }
        #expect(agents.count == 1)
        #expect(agents.agents.first?.row == "sub:a")
    }

    // MARK: - The Mac's way in (ov-641)

    /// A pane as the Mac reads it from its own fleet.
    static func pane(
        _ id: String, orchestrator: Bool = false, names: [String] = [], ids: [String] = [], running: Int? = nil
    ) -> AgentPane {
        AgentPane(
            terminal: id, worktree: "wt-\(id)", label: "claude", isOrchestrator: orchestrator, workspace: main,
            activity: "Reading", startedMs: nil, mark: GlanceMark(attention: .quiet, core: .producing),
            subagents: names, subagentIds: ids, subagentsRunning: running)
    }

    @Test("Panes a client read itself count as the fleet's do: the larger of the count and the names, the orchestrator never listed")
    func theMacsPanes() {
        let panes = [
            Self.pane("orch", orchestrator: true, names: ["Survey the repo"], ids: ["toolu_s"], running: 3),
            Self.pane("lane", names: ["Review ov-616"], ids: ["toolu_r"], running: 1),
        ]
        let rows = [Self.entry("sub:s", "Explore", asked: "Survey the repo", call: "toolu_s")]
        let agents = RunningAgents.workspace(Self.main, runner: Self.runner, panes: panes) { $0 == "orch" ? rows : nil }
        // The orchestrator's three (one named, two in the background), the lane, and the lane's one.
        #expect(agents.count == 5)
        #expect(agents.agents.map(\.name) == ["Explore", "2 more agents", "claude", "Subagent"])
        #expect(agents.agents.first?.opensOwnConversation == true)
        #expect(agents.agents.first { $0.kind == .pane }?.worktree == "wt-lane")
        #expect(RunningAgents.workspace(Self.main, runner: Self.runner, panes: []).count == 0)
    }
}
