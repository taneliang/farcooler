import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The agents card and its panel on the Mac (ov-641). The counting rule is
/// AgentKit's (`RunningAgentsTests`, over panes a client reads itself); this
/// holds the Mac's way in to it: the CLI's JSON through the Mac's own
/// `Terminal`, a fleet that every refresh and every event replaces, the
/// views, and where a row goes.
@MainActor
@Suite(.serialized)
struct WorkspaceAgentsTests {
    typealias P = NativePolishTests

    static let main = "ws-main"

    /// One terminal, as `farcooler worktree list --json` writes it.
    static func terminal(
        _ id: String, title: String = "claude", role: String = "agent", state: String = "running",
        activity: String = "working", line: String? = nil, subagents: [String] = [], running: Int? = nil,
        ids: [String]? = nil, workspace: String? = main, turn: Double? = 2000, failed: Bool = false
    ) -> String {
        var fields = [
            #""id":"\#(id)","short":"\#(id)","title":"\#(title)","preset":"claude","state":"\#(state)","activity":"\#(activity)""#,
            #""activitySince":1000,"epoch":0,"role":"\#(role)""#,
            #""subagents":[\#(subagents.map { "\"\($0)\"" }.joined(separator: ","))]"#,
        ]
        if let line { fields.append(#""line":"\#(line)""#) }
        if let running { fields.append(#""subagentsRunning":\#(running)"#) }
        if let ids { fields.append(#""subagentIds":[\#(ids.map { "\"\($0)\"" }.joined(separator: ","))]"#) }
        if let workspace { fields.append(#""workspace":"\#(workspace)""#) }
        if failed { fields.append(#""turnFailed":true"#) }
        if let turn { fields.append(#""turnStartedAt":\#(turn)"#) }
        return "{\(fields.joined(separator: ","))}"
    }

    static func worktree(_ id: String, host: String = "", _ terminals: [String]) -> String {
        #"{"id":"\#(id)","short":"\#(id)","repository":"demo","state":"active","task":"\#(id)","branch":"b/\#(id)","host":"\#(host)","worktree":"/tmp/\#(id)","terminals":[\#(terminals.joined(separator: ","))]}"#
    }

    static func fleet(_ worktrees: [String]) throws -> Fleet {
        let json = #"{"runtime_healthy":true,"live_panes":1,"worktrees":[\#(worktrees.joined(separator: ","))]}"#
        return try JSONDecoder().decode(Fleet.self, from: Data(json.utf8))
    }

    /// Main's orchestrator with two named subagents and a third in the
    /// background, a lane agent with one, a shell and an exited agent, and an
    /// agent in another workspace and on another runner.
    static func busy(turn: Double = 2000) throws -> Fleet {
        try fleet([
            worktree("checkout", [
                terminal(
                    "orch", role: "orchestrator", subagents: ["Survey the repo", "Review ov-616"], running: 3,
                    ids: ["toolu_s", "toolu_r"]),
                terminal("shell", role: "shell", activity: "none"),
            ]),
            worktree("lane", [
                terminal("builder", title: "Build ov-619", line: "Edit RunningAgents.swift", subagents: ["Survey the repo"], turn: turn),
                terminal("gone", state: "exited"),
            ]),
            worktree("bill", [terminal("biller", workspace: "ws-billing")]),
            worktree("away", host: "other", [terminal("remote")]),
        ])
    }

    static func entry(_ id: String, _ title: String, asked: String, call: String? = nil, started: Int64? = nil) -> AgentTray.Entry {
        AgentTray.Entry(
            id: id, isMain: false, title: title, description: asked, action: "Read x", running: true, startedMs: started,
            agentId: "agent-\(id)", toolUseId: call)
    }

    // MARK: - The count

    @Test("The CLI's fleet carries how many subagents run and the call id behind each name")
    func theCLIsFleetDecodes() throws {
        let fleet = try Self.busy()
        let orchestrator = try #require(fleet.worktrees.first?.terminals.first)
        #expect(orchestrator.subagentsRunning == 3)
        #expect(orchestrator.subagentIds == ["toolu_s", "toolu_r"])
        // A CLI before the keys: neither, and the names are the count.
        let old = try #require(Self.fleet([Self.worktree("w", [Self.terminal("t", subagents: ["a"])])]).worktrees.first?.terminals.first)
        #expect(old.subagentsRunning == nil && old.subagentIds == nil)
    }

    @Test("The Mac counts as the iPhone does: the larger of the count and the names, over a workspace's agent panes on one runner")
    func theCount() throws {
        let agents = MacAgents.running(workspace: Self.main, host: "", fleet: try Self.busy())
        // The orchestrator's three (two named, one unnamed), the lane, and the lane's one; no shell, no exited
        // pane, nothing of Billing's or another runner's.
        #expect(agents.count == 5)
        #expect(agents.agents.map(\.name) == ["Subagent", "Subagent", "1 more agent", "Build ov-619", "Subagent"])
        #expect(agents.agents.map(\.kind) == [.subagent, .subagent, .subagent, .pane, .subagent])
        #expect(agents.summary == "5 agents")
        let lane = try #require(agents.agents.first { $0.kind == .pane })
        #expect(lane.worktree == "lane" && lane.terminal == "builder" && lane.activity == "Edit RunningAgents.swift")
        #expect(lane.startedMs == 2000, "its turn's start")
        #expect(MacAgents.running(workspace: "ws-billing", host: "", fleet: try Self.busy()).count == 1, "Billing's own agent pane, and only it")
    }

    @Test("The Mac's count is AgentKit's on the same fleet: every case in the shared fixture, on every runner")
    func theCountIsAgentKits() throws {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/running-agents-cases.json"))
        let object = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        let fleet = try JSONDecoder().decode(
            Fleet.self, from: JSONSerialization.data(withJSONObject: try #require(object["fleet"])))
        let cases = try #require(object["cases"] as? [[String: Any]])
        #expect(cases.count >= 5)
        for c in cases {
            let workspace = try #require(c["workspace"] as? String)
            let host = try #require(c["host"] as? String)
            let expected = try #require(c["count"] as? Int)
            let counted = MacAgents.running(workspace: workspace, host: host, fleet: fleet).count
            #expect(counted == expected, "\(workspace) on \(host.isEmpty ? "this runner" : host): \(c["why"] as? String ?? "")")
        }
    }

    @Test("Agents a turn left running in the background are counted after it ends, and an empty list of names does not hide them")
    func backgroundAgentsOutliveTheirTurn() throws {
        let fleet = try Self.fleet([Self.worktree("checkout", [Self.terminal("orch", role: "orchestrator", running: 3)])])
        let agents = MacAgents.running(workspace: Self.main, host: "", fleet: fleet)
        #expect(agents.count == 3, "the card collapsed while three agents ran")
        #expect(agents.agents.map(\.name) == ["3 more agents"])
    }

    @Test("A conversation this window holds adds each subagent's type and start, joined on the call id")
    func rowsJoinOnTheCallId() throws {
        let rows = [
            Self.entry("sub:y", "Explore", asked: "Something else", call: "toolu_y"),
            Self.entry("sub:r", "General purpose", asked: "Review ov-616", call: "toolu_r", started: 5_000),
        ]
        let agents = MacAgents.running(workspace: Self.main, host: "", fleet: try Self.busy()) { $0 == "orch" ? rows : nil }
        let review = try #require(agents.agents.first { $0.row == "sub:r" })
        #expect(review.name == "General purpose" && review.startedMs == 5_000 && review.opensOwnConversation)
        #expect(agents.count == 5, "a row no name accounts for adds nothing")
        #expect(!agents.agents.contains { $0.row == "sub:y" && $0.activity == "Something else" } || agents.count == 5)
    }

    @Test("The count is right across reconnects: a fresh read replaces the fleet whole, and the push keeps the running count")
    func acrossReconnects() async throws {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        func read(_ worktrees: [String]) async {
            let body = #"{"runtime_healthy":true,"live_panes":1,"worktrees":["# + worktrees.joined(separator: ",") + "]}"
            client.commandRunnerForTesting = { (args: [String]) -> (data: Data?, message: String?) in
                if args.prefix(2) == ["worktree", "list"] { return (Data(body.utf8), nil) }
                return (nil, "not in this test")
            }
            await client.refresh()
        }
        func count() -> Int { MacAgents.running(workspace: Self.main, host: "", fleet: client.fleet).count }

        await read([Self.worktree("checkout", [Self.terminal("orch", role: "orchestrator", subagents: ["a", "b"], running: 2, ids: ["x", "y"])])])
        #expect(count() == 2)

        // The turn ends; the two it started in the background run on. The push says so.
        let ended = try JSONDecoder().decode(
            TerminalEvent.self,
            from: Data(
                #"{"id":"orch","short":"orch","worktree":"checkout","title":"claude","preset":"claude","state":"running","activity":"done","subagents":[],"subagentsRunning":2,"subagentIds":[],"workspace":"ws-main","role":"orchestrator"}"#
                    .utf8))
        client.apply(ended)
        #expect(count() == 2, "the names went with the turn; the agents did not")

        // The link drops and comes back; the runner says one finished meanwhile.
        await read([Self.worktree("checkout", [Self.terminal("orch", role: "orchestrator", running: 1)])])
        #expect(count() == 1)
        // Back again, and the runner has none.
        await read([Self.worktree("checkout", [Self.terminal("orch", role: "orchestrator", running: 0)])])
        #expect(count() == 0)
    }

    // MARK: - Where a row goes

    @Test("An agent opens its pane in its worktree; the orchestrator's own and its subagents open the conversation column")
    func destinations() throws {
        let agents = MacAgents.running(workspace: Self.main, host: "", fleet: try Self.busy())
        let lane = try #require(agents.agents.first { $0.kind == .pane })
        let laneSub = try #require(agents.agents.last)
        let orchestratorSub = try #require(agents.agents.first)
        let at = { (agent: RunningAgent) in ContentView.destination(of: agent, seat: "orch", host: "", workspace: Self.main) }
        #expect(at(lane) == .workspace(host: "", workspace: Self.main, focus: .worktree("lane", terminal: "builder")))
        #expect(at(laneSub) == at(lane), "a subagent opens its parent's pane")
        #expect(at(orchestratorSub) == .workspace(host: "", workspace: Self.main, focus: nil))
    }

    // MARK: - Drawn

    static func strip(width: CGFloat = 700) -> some View {
        WorkspaceStripRow {
            Text("Now and Next").frame(maxWidth: .infinity, minHeight: 40, alignment: .leading).probed("now-next-stub")
        }
        .frame(width: width, alignment: .topLeading)
    }

    static func drawn(_ view: some View, width: CGFloat = 700, height: CGFloat = 400) async -> (P.Frames, NSWindow) {
        let frames = P.Frames()
        let window = P.window(width: width, height: height, P.Framed(frames: frames, content: view.frame(width: width, height: height, alignment: .topLeading)))
        await NativeAgentTests.settle(window, still: frames)
        return (frames, window)
    }

    @Test("Folded, Now/Next is the column's width less the paper's inset each side, and no agents card shares its row (ov-829)")
    func theStrip() async throws {
        let (frames, window) = await Self.drawn(Self.strip(width: 520), width: 520)
        defer { window.close() }
        let now = try #require(frames.views["now-next-stub"], "\(frames.views.keys.sorted())")
        #expect(abs(now.minX - Pane.inset) < 0.5 && abs(now.maxX - (520 - Pane.inset)) < 0.5, "the paper's edges: \(now)")
        #expect(abs(now.minY - Pane.inset) < 0.5, "the paper's gap above: \(now)")
        #expect(frames.views["agents-card"] == nil)
    }

    @Test("The panel lists the orchestrator's subagents under their heading, then each agent, a row each that opens it")
    func thePanel() async throws {
        let agents = MacAgents.running(workspace: Self.main, host: "", fleet: try Self.busy())
        var chosen: [String] = []
        let (frames, window) = await Self.drawn(AgentsPanel(agents: agents) { chosen.append($0.name) }, width: 420, height: 560)
        defer { window.close() }
        let ids = Set(frames.views.keys)
        for id in ["agent-row-builder", "agent-row-builder-Survey the repo#0", "agent-row-orch-Survey the repo#0", "agent-row-orch-more"] { #expect(ids.contains(id), "\(id) in \(ids.sorted())") }
        #expect(ids.filter { $0.hasPrefix("agent-row-orch") }.count == 3, "the orchestrator's three, each its own: \(ids.sorted())")
        let sections = AgentsPanel.sections(agents)
        #expect(sections.map(\.title) == [AgentsWords.orchestratorSection, AgentsWords.panesSection])
        #expect(sections.map { $0.agents.count } == [3, 2])
        #expect(P.click("agent-row-builder", frames: frames, in: window))
        await NativeAgentTests.settle(window, 200)
        #expect(chosen == ["Build ov-619"])
        let (none, noneWindow) = await Self.drawn(AgentsPanel(agents: .empty) { _ in }, width: 420, height: 300)
        defer { noneWindow.close() }
        #expect(none.views["agents-none"] != nil && none.views["agents-list"] == nil)
    }
}
