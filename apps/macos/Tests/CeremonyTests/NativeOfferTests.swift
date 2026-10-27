import AgentKit
import AppKit
import Foundation
import SwiftUI
import Testing

@testable import Far_Cooler

/// Which panes are offered the conversation view, and the reason a pane
/// isn't (ov-443). The owner's orchestrator was a claude typed into a shell,
/// launched as `shell` and labeled with its session's title, so it was never
/// offered the view and nothing said why.
@MainActor
@Suite(.serialized)
struct NativeOfferTests {
    static func terminal(_ fields: String, id: String = "0199aaaa-0000-7000-8000-0000000000a1") throws -> Terminal {
        try JSONDecoder().decode(
            Terminal.self,
            from: Data(#"{"id":"\#(id)","short":"t","title":"Terminal 1","state":"running","epoch":1,"paneMode":"terminal",\#(fields)}"#.utf8))
    }

    static func agents(offered: Set<String>? = ["agent_rows", "agent_compose"], pairing: RemotePairing? = nil, target: String = "")
        -> NativeAgents
    {
        let defaults = ScratchDefaults.suite("native-offer-tests-\(UUID().uuidString)")!
        let agents = NativeAgents(defaults: defaults, pairing: pairing ?? Self.pairing(defaults))
        if let offered {
            agents.pretend(rowsServed: NativeAgents.serves(offered), core: RunnerCore(), offered: offered, target: target)
        } else {
            agents.pretend(rowsServed: false, core: nil, target: target)
        }
        return agents
    }

    static func pairing(_ defaults: UserDefaults) -> RemotePairing {
        RemotePairing(key: ConversationKey(vault: MemoryConversationKeyVault()), defaults: defaults)
    }

    /// Goes red with `isAgentInATerminal` back on `program ?? preset`.
    @Test("A claude typed into a shell, or named after its session, is offered the view")
    func theRunningAgentIsOffered() throws {
        let agents = Self.agents()
        let typed = try Self.terminal(#""preset":"User test issues","program":"shell","runningAgent":"claude""#)
        let titled = try Self.terminal(#""preset":"Fix the login bug","program":"claude","runningAgent":"claude""#)
        let shell = try Self.terminal(#""preset":"fish","program":"shell""#)
        let quit = try Self.terminal(#""preset":"fish","program":"shell","runningAgent":null"#)
        #expect(agents.offers(typed, target: ""))
        #expect(agents.offers(titled, target: ""))
        #expect(!agents.offers(shell, target: ""))
        #expect(!agents.offers(quit, target: ""), "a claude that quit to its shell")
        // A pane launched as claude whose claude has exited: the runner says
        // nothing runs (empty), which beats the program it was launched as.
        let exited = try Self.terminal(#""preset":"claude","program":"claude","runningAgent":"""#)
        #expect(!agents.offers(exited, target: ""), "an exited claude is offered by what it was launched as")
        #expect(agents.unavailable(exited, target: "") == .notAnAgent)
        #expect(NativeAgents.agent(of: typed) == "claude")
        #expect(agents.unavailable(typed, target: "") == nil)
    }

    @Test("A pane that isn't offered the view says why")
    func eachReasonIsSaid() throws {
        let claude = try Self.terminal(#""preset":"claude","program":"claude""#)
        let shell = try Self.terminal(#""preset":"fish","program":"shell""#)
        #expect(Self.agents().unavailable(shell, target: "") == .notAnAgent)
        #expect(Self.agents(offered: nil).unavailable(shell, target: "") == .notAnAgent, "a shell is never told about its runner")
        // A runner from before ov-830 with its projector setting off.
        #expect(Self.agents(offered: ["agent_compose", "projector_setting"]).unavailable(claude, target: "") == .runnerNeedsUpdate)
        #expect(Self.agents(offered: nil).unavailable(claude, target: "") == .unreachable)

        // A remote runner the person unpaired here.
        let defaults = ScratchDefaults.suite("native-offer-pairing-\(UUID().uuidString)")!
        defaults.set(["box": "unpaired"], forKey: RemotePairing.recordKey)
        let unpaired = Self.agents(offered: nil, pairing: Self.pairing(defaults), target: "box")
        #expect(unpaired.unavailable(claude, target: "box") == .pairingNeeded)
        #expect(AgentConversation.Unavailable.pairingNeeded.sentence.contains("Pair"))
    }

    /// The pane's switch, dimmed, for a reason about the runner; none for a
    /// shell.
    @Test("A Claude pane whose runner can't serve the view shows a dimmed switch; a shell shows none")
    func theDimmedSwitch() async throws {
        let claude = try Self.terminal(#""preset":"Fix the login bug","program":"shell","runningAgent":"claude""#)
        let shell = try Self.terminal(#""preset":"fish","program":"shell""#)
        let old: Set<String> = ["agent_compose"]
        let cases: [(String, NativeAgents, Terminal, Bool)] = [
            ("a runner that needs an update", Self.agents(offered: old), claude, true),
            ("a shell", Self.agents(offered: old), shell, false),
            ("a runner not reached", Self.agents(offered: nil), claude, true),
            ("offered", Self.agents(), claude, false),
        ]
        for (why, agents, terminal, dimmed) in cases {
            let seen = NativeAgentTests.Seen()
            let life = NativeAgentTests.SurfaceLife()
            let window = NativeAgentTests.window(NativeAgentTests.Probe(seen: seen, content: NativeSwitch(terminal: terminal, target: "", isFocused: true, agents: agents) { focused in
                NativeAgentTests.StandInSurface(life: life, focused: focused)
            }))
            await NativeAgentTests.settle(window)
            #expect(seen.ids.contains("native-switch-unavailable") == dimmed, "\(why)")
            #expect(seen.ids.contains("terminal-surface"), "\(why)")
            window.close()
        }
        #expect(NativeSwitchReason.shown(.notAnAgent) == nil)
        #expect(NativeSwitchReason.shown(.runnerNeedsUpdate) == .runnerNeedsUpdate)
        #expect(ConversationUnavailableChip.settingsTab(for: .pairingNeeded) == "devices")
    }

    /// The help, and the line under the item that's read without hovering
    /// (ov-830), naming the check that failed.
    @Test("The menu item says why it's dimmed, on a line of its own, and nothing when it acts")
    func theMenuSaysWhy() {
        let reason = AgentConversation.Unavailable.runnerNeedsUpdate
        let dimmed = LayoutMenuFocus.make(
            group: nil, here: nil, layouts: [], switchesConversation: false,
            conversationUnavailable: reason.sentence, conversationNote: reason.note)
        #expect(dimmed.conversationUnavailable == "This runner needs an update to show the conversation.")
        #expect(dimmed.conversationNote == "Runner version: This runner needs an update to show the conversation.")
        let acts = LayoutMenuFocus.make(
            group: nil, here: nil, layouts: [], switchesConversation: true, conversationUnavailable: "stale", conversationNote: "stale")
        #expect(acts.conversationUnavailable == nil)
        #expect(acts.conversationNote == nil)
    }

    // MARK: - An adopted orchestrator (ov-830)

    /// The owner's orchestrator, as the runner lists it: claude typed into a
    /// shell pane (`program` shell, labeled with its session's title), then
    /// adopted (`role` orchestrator), with no session the runner launched it
    /// on. Offered the view on a runner that serves it, by nothing but the
    /// runner and the agent: no setting stands in the way. Goes red with a
    /// setting back in `offers` (`guard enabled`) or the agent read off
    /// `program`.
    @Test("An orchestrator adopted from a claude typed into a shell is offered the view")
    func anAdoptedOrchestratorIsOffered() throws {
        let adopted = try Self.terminal(
            #""preset":"User test issues and onboarding flow (2)","program":"shell","runningAgent":"claude","role":"orchestrator","agentSessionId":null"#)
        let agents = Self.agents()
        #expect(adopted.role == "orchestrator")
        #expect(agents.offers(adopted, target: ""))
        #expect(agents.unavailable(adopted, target: "") == nil)
        // Fresh from `init`, as on a Mac that never had a setting: the runner
        // reached is the only gate.
        let fresh = NativeAgents(defaults: ScratchDefaults.suite("native-offer-fresh-\(UUID().uuidString)")!)
        #expect(!fresh.offers(adopted, target: ""), "no runner reached yet")
        #expect(fresh.unavailable(adopted, target: "") == .unreachable)
        fresh.pretend(rowsServed: true, core: RunnerCore(), offered: ["agent_rows", "agent_compose"])
        #expect(fresh.offers(adopted, target: ""))
    }

    /// The regression: an orchestrator the runner launched as claude, with
    /// its session, is still offered the view.
    @Test("An orchestrator the runner launched is still offered the view")
    func aLaunchedOrchestratorIsStillOffered() throws {
        let launched = try Self.terminal(
            #""preset":"claude","program":"claude","runningAgent":"claude","role":"orchestrator","agentSessionId":"5f0e307b-0000-4000-8000-000000000001""#)
        #expect(Self.agents().offers(launched, target: ""))
        #expect(Self.agents().unavailable(launched, target: "") == nil)
    }

    /// The reason a dimmed orchestrator menu shows, read without hovering
    /// (ov-830): the check, then the sentence.
    @Test("The orchestrator menu's dimmed item carries its reason as a line")
    func theOrchestratorMenuSaysWhyInline() throws {
        let adopted = try Self.terminal(#""preset":"Fix the login bug","program":"shell","runningAgent":"claude","role":"orchestrator""#)
        let old = Self.agents(offered: ["agent_compose", "projector_setting"])
        let reason = try #require(old.unavailable(adopted, target: ""))
        #expect(reason.isAboutTheRunner)
        #expect(reason.note == "Runner version: This runner needs an update to show the conversation.")
    }

    // MARK: - The real CLI

    /// The real CLI's bytes through the Mac's decoder: a program named
    /// `claude` (a link to `sleep`, never the real one) typed into a shell
    /// pane is listed as running claude, and offered the view.
    @Test(
        "A claude typed into a shell is offered the view, through the real CLI and daemon",
        .enabled(if: PaneHeaderProgramTests.runnable))
    func theRealCLINamesTheRunningAgent() async throws {
        let home = "/tmp/fco-\(UUID().uuidString.prefix(6))"
        defer { try? FileManager.default.removeItem(atPath: home) }
        let demo = home + "/repos/demo"
        let bin = home + "/bin"
        try FileManager.default.createDirectory(atPath: demo, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(atPath: bin, withIntermediateDirectories: true)
        // A link, not a copy: a copied system binary is killed for its signature.
        try FileManager.default.createSymbolicLink(atPath: bin + "/claude", withDestinationPath: "/bin/sleep")
        for args in [["init", "-q"], ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "i"]] {
            _ = await ProcessRunner.run("/usr/bin/git", ["-C", demo] + args, deadline: 30)
        }
        let cli = try #require(PaneHeaderProgramTests.cli)
        func farcooler(_ args: [String]) async -> ProcessRunner.Result {
            var environment = ProcessInfo.processInfo.environment
            ScratchDaemon.isolate(&environment, home: home + "/h")
            return await ProcessRunner.run(cli, args, environment: environment, deadline: 60)
        }
        func tearDown() async { await ScratchDaemon.stop(cli: cli, farcoolerHome: home + "/h") }
        do {
            // Each setup call must answer: one that timed out or failed left
            // stdout empty, and the next step then failed with "Unable to
            // parse empty data" and none of the CLI's reason (ov-801).
            try await farcooler(["--json", "daemon", "ensure"]).mustSucceed("daemon ensure")
            try await farcooler(["root", "add", home + "/repos"]).mustSucceed("root add")
            try await farcooler(["repo", "register", demo]).mustSucceed("repo register")
            let made = await farcooler(["--json", "worktree", "create", "demo", "wt1", "--branch", "wt1", "--fork-only"])
            let worktree = try #require(try made.jsonObject("worktree create")["short"] as? String)
            let created = await farcooler(["--json", "terminal", "create", worktree])
            let terminal = try #require(try created.jsonObject("terminal create")["id"] as? String)
            try await farcooler(["terminal", "send", terminal, "\(bin)/claude 600\r"]).mustSucceed("terminal send")
            // Until it says claude, not until it says anything: the runner
            // says "" for the shell the pane starts as, and on a slow host
            // (CI's VM) the first sample lands before the shell has read the
            // typed line. Stopping on that "" read a shell, not the detection.
            var listed: Terminal?
            var lastList = "no list yet"
            for _ in 0..<60 {
                let list = await farcooler(["--json", "worktree", "list"])
                lastList = list.account
                if let fleet = try? list.decoded(Fleet.self, "worktree list") {
                    listed = fleet.worktrees.flatMap(\.terminals).first { $0.id == terminal }
                }
                if listed?.runningAgent == "claude" { break }
                try await Task.sleep(for: .milliseconds(500))
            }
            let pane = try #require(listed)
            #expect(
                pane.runningAgent == "claude",
                "the runner never said claude runs here in 30 s; last said \(pane.runningAgent.map { "\"\($0)\"" } ?? "nothing"); last list: \(lastList)")
            #expect(pane.program == "shell")
            #expect(Self.agents().offers(pane, target: ""))
        } catch {
            await tearDown()
            throw error
        }
        await tearDown()
    }
}
