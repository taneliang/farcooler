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

    static func agents(enabled: Bool = true, offered: Set<String>? = ["agent_rows", "agent_compose"], pairing: RemotePairing? = nil, target: String = "")
        -> NativeAgents
    {
        let defaults = UserDefaults(suiteName: "native-offer-tests-\(UUID().uuidString)")!
        let agents = NativeAgents(defaults: defaults, pairing: pairing ?? Self.pairing(defaults))
        if let offered {
            agents.pretend(enabled: enabled, rowsServed: NativeAgents.serves(offered), core: RunnerCore(), offered: offered, target: target)
        } else {
            agents.pretend(enabled: enabled, rowsServed: false, core: nil, target: target)
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
        #expect(NativeAgents.agent(of: typed) == "claude")
        #expect(agents.unavailable(typed, target: "") == nil)
    }

    @Test("A pane that isn't offered the view says why")
    func eachReasonIsSaid() throws {
        let claude = try Self.terminal(#""preset":"claude","program":"claude""#)
        let shell = try Self.terminal(#""preset":"fish","program":"shell""#)
        #expect(Self.agents().unavailable(shell, target: "") == .notAnAgent)
        #expect(Self.agents(enabled: false).unavailable(shell, target: "") == .notAnAgent, "a shell is never told to turn a setting on")
        #expect(Self.agents(enabled: false).unavailable(claude, target: "") == .settingOff)
        #expect(Self.agents(offered: ["agent_compose", "projector_setting"]).unavailable(claude, target: "") == .runnerNeedsUpdate)
        #expect(Self.agents(offered: nil).unavailable(claude, target: "") == .unreachable)

        // A remote runner the person unpaired here.
        let defaults = UserDefaults(suiteName: "native-offer-pairing-\(UUID().uuidString)")!
        defaults.set(["box": "unpaired"], forKey: RemotePairing.recordKey)
        let unpaired = Self.agents(offered: nil, pairing: Self.pairing(defaults), target: "box")
        #expect(unpaired.unavailable(claude, target: "box") == .pairingNeeded)
        #expect(AgentConversation.Unavailable.pairingNeeded.sentence.contains("Pair"))
    }

    /// The pane's switch, dimmed, for a reason about the runner; none for a
    /// shell, and none for the setting being off, which is a choice.
    @Test("A Claude pane whose runner can't serve the view shows a dimmed switch; a shell shows none")
    func theDimmedSwitch() async throws {
        let claude = try Self.terminal(#""preset":"Fix the login bug","program":"shell","runningAgent":"claude""#)
        let shell = try Self.terminal(#""preset":"fish","program":"shell""#)
        let old: Set<String> = ["agent_compose"]
        let cases: [(String, NativeAgents, Terminal, Bool)] = [
            ("a runner that needs an update", Self.agents(offered: old), claude, true),
            ("a shell", Self.agents(offered: old), shell, false),
            ("the setting off", Self.agents(enabled: false), claude, false),
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
        #expect(NativeSwitchReason.shown(.settingOff) == nil)
        #expect(NativeSwitchReason.shown(.notAnAgent) == nil)
        #expect(NativeSwitchReason.shown(.runnerNeedsUpdate) == .runnerNeedsUpdate)
        #expect(ConversationUnavailableChip.settingsTab(for: .pairingNeeded) == "devices")
    }

    @Test("The menu item's help says why it's dimmed, and nothing when it acts")
    func theMenuSaysWhy() {
        let dimmed = LayoutMenuFocus.make(
            group: nil, here: nil, layouts: [], switchesMode: false, switchesConversation: false,
            conversationUnavailable: AgentConversation.Unavailable.settingOff.sentence)
        #expect(dimmed.conversationUnavailable == "Turn on Conversation view in Settings.")
        let acts = LayoutMenuFocus.make(
            group: nil, here: nil, layouts: [], switchesMode: false, switchesConversation: true, conversationUnavailable: "stale")
        #expect(acts.conversationUnavailable == nil)
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
        func farcooler(_ args: [String]) async -> (out: Data, err: String) {
            var environment = ProcessInfo.processInfo.environment
            ScratchDaemon.isolate(&environment, home: home + "/h")
            let ran = await ProcessRunner.run(cli, args, environment: environment, deadline: 60)
            return (ran.stdout, String(decoding: ran.stderr, as: UTF8.self))
        }
        _ = await farcooler(["--json", "daemon", "ensure"])
        func tearDown() async { await ScratchDaemon.stop(cli: cli, farcoolerHome: home + "/h") }
        do {
            _ = await farcooler(["root", "add", home + "/repos"])
            _ = await farcooler(["repo", "register", demo])
            let made = await farcooler(["--json", "worktree", "create", "demo", "wt1", "--branch", "wt1", "--fork-only"])
            let worktree = try #require(
                (try JSONSerialization.jsonObject(with: made.out) as? [String: Any])?["short"] as? String, "\(made.err)")
            let created = await farcooler(["--json", "terminal", "create", worktree])
            let terminal = try #require(
                (try JSONSerialization.jsonObject(with: created.out) as? [String: Any])?["id"] as? String, "\(created.err)")
            _ = await farcooler(["terminal", "send", terminal, "\(bin)/claude 600\r"])
            var listed: Terminal?
            for _ in 0..<60 {
                let list = await farcooler(["--json", "worktree", "list"])
                let fleet = try JSONDecoder().decode(Fleet.self, from: list.out)
                listed = fleet.worktrees.flatMap(\.terminals).first { $0.id == terminal }
                if listed?.runningAgent != nil { break }
                try await Task.sleep(for: .milliseconds(500))
            }
            let pane = try #require(listed)
            #expect(pane.runningAgent == "claude", "the runner never said claude runs here")
            #expect(pane.program == "shell")
            #expect(Self.agents().offers(pane, target: ""))
        } catch {
            await tearDown()
            throw error
        }
        await tearDown()
    }
}
