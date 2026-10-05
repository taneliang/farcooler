import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// What the owner's actions on a ruling send (ov-333): Keep is the runner's
/// own mark and never reaches the orchestrator; Reverse sends the ruling's
/// recorded reversal to its chat; Discuss leaves a quote in its composer,
/// unsent. Read off the CLI calls the app makes, through the same
/// `DaemonClient` the window uses.
@MainActor
struct PlanRulingOwnerActionsTests {
    static let ruling = PlanRuling(
        id: "r2", number: 2, decision: "The inbox is amber.", why: "One attention color.",
        reversal: "One token; every surface follows.")

    private static func seat(chat: Bool = true) -> BoardPane {
        var terminal = Terminal(id: "orch", short: "orch", title: "orch", preset: "claude", state: "running", epoch: 0)
        terminal.paneMode = chat ? "agent" : "terminal"
        terminal.role = "orchestrator"
        let worktree = Worktree(
            id: "w", short: "w", task: "w", branch: "main", repository: "shop", host: "", path: "/tmp/w",
            state: "active", terminals: [terminal], repositoryID: "repo", workspace: "ws")
        return BoardPane(terminal: terminal, worktree: worktree)
    }

    /// A client whose every CLI call is recorded and answered.
    private static func client(_ calls: PlanViewTests.Calls, answering: Bool = true) -> DaemonClient {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            calls.args.append(args)
            return answering ? (Data("{}".utf8), nil) : (nil, "no")
        }
        return client
    }

    @Test("Keep runs `plan ruling keep` as the owner and sends nothing to any terminal")
    func keepIsLocal() async throws {
        let calls = PlanViewTests.Calls()
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        calls.args.removeAll()
        let ruling = try #require(store.plan.plan.rulings.first { $0.short == "R-2" })
        #expect(await store.plan.keep(ruling))
        let keeps = calls.args.filter { $0.starts(with: ["plan", "ruling", "keep"]) }
        #expect(keeps.count == 1)
        let keep = try #require(keeps.first)
        #expect(keep.prefix(4) == ["plan", "ruling", "keep", "R-2"])
        #expect(keep.contains("--actor") && keep.last == "user", "as the owner, whatever the environment says")
        #expect(!calls.args.contains { $0.first == "terminal" }, "Keep never reaches the orchestrator: \(calls.args)")
    }

    @Test("Keep shows the ruling kept at once, and Keep All every open one")
    func keepLandsInstantly() async throws {
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults())
        await store.plan.reload()
        let before = store.plan.plan
        #expect(before.openRulings.map(\.short) == ["R-2"])
        store.plan.showKept(Set(before.openRulings.map(\.id)))
        #expect(store.plan.plan.openRulings.isEmpty)
        #expect(store.plan.plan.pastRulings.map(\.short).first == "R-2", "the newest settled leads Past Decisions")
        #expect(store.plan.plan.pastRulings.first?.settledBy == "user")
    }

    @Test("Keep All is one `plan ruling keep --all`, and does nothing with nothing open")
    func keepAllIsOneCall() async throws {
        let calls = PlanViewTests.Calls()
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        calls.args.removeAll()
        #expect(await store.plan.keepAll())
        let keeps = calls.args.filter { $0.starts(with: ["plan", "ruling", "keep"]) }
        #expect(keeps.count == 1 && keeps[0].contains("--all") && keeps[0].last == "user", "\(keeps)")
        let empty = PlanViewTests.Calls()
        empty.plan = try PlanViewTests.emptyPlan()
        let none = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: empty)
        await none.plan.reload()
        empty.args.removeAll()
        #expect(await none.plan.keepAll() == false)
        #expect(!empty.args.contains { $0.starts(with: ["plan", "ruling"]) })
    }

    @Test("A runner without board_ruling_actions is never asked to keep")
    func oldRunnerKeepsNothing() async throws {
        let calls = PlanViewTests.Calls()
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        store.client.daemonBuild = DaemonBuild(
            version: "test", matches: true, platform: "macos",
            capabilities: Set(Capability.allCases.map(\.rawValue).filter { $0 != "board_ruling_actions" }))
        calls.args.removeAll()
        let ruling = try #require(store.plan.plan.rulings.first { $0.short == "R-2" })
        #expect(await store.plan.keep(ruling) == false)
        #expect(await store.plan.keepAll() == false)
        #expect(calls.args.isEmpty)
    }

    @Test("Reverse sends the ruling's recorded reversal to a chat orchestrator, and leaves the composer alone")
    func reverseSendsTheReversal() async {
        let calls = PlanViewTests.Calls()
        let handoff = ComposerHandoff()
        let outcome = await RulingOrchestrator.reverse(Self.ruling, seat: Self.seat(), client: Self.client(calls))
        #expect(outcome == .sent)
        let sends = calls.args.filter { $0.starts(with: ["terminal", "agent-prompt", "orch"]) }
        #expect(sends.count == 1)
        let text = sends.first?.last ?? ""
        #expect(text.contains("One token; every surface follows."), "the recorded reversal: \(text)")
        #expect(text.contains("`plan ruling reverse R-2 --sha <commit>`"))
        #expect(!calls.args.contains { $0.first == "plan" }, "reversing never marks the ruling itself")
        #expect(handoff.waiting.isEmpty)
    }

    @Test("Reverse into a terminal orchestrator is typed with no Enter, and a refusal copies it")
    func reverseIntoATerminal() async {
        let calls = PlanViewTests.Calls()
        let outcome = await RulingOrchestrator.reverse(Self.ruling, seat: Self.seat(chat: false), client: Self.client(calls))
        #expect(outcome == .drafted)
        #expect(calls.args.contains { $0.starts(with: ["terminal", "draft-prompt", "orch"]) })
        #expect(!calls.args.contains { $0.starts(with: ["terminal", "agent-prompt"]) })
        let refused = PlanViewTests.Calls()
        let copied = Self.client(refused, answering: false)
        var clipboard: [String] = []
        copied.copyToClipboard = { clipboard.append($0) }
        #expect(await RulingOrchestrator.reverse(Self.ruling, seat: Self.seat(chat: false), client: copied) == .copied)
        #expect(clipboard.count == 1 && clipboard[0].contains("One token"))
    }

    @Test("Discuss leaves a quote in the composer, unsent")
    func discussQuotesUnsent() async {
        let calls = PlanViewTests.Calls()
        let handoff = ComposerHandoff()
        let delivery = await RulingOrchestrator.discuss(
            Self.ruling, seat: Self.seat(), client: Self.client(calls), handoff: handoff)
        #expect(delivery == .composer)
        #expect(handoff.waiting["orch"] == "About ruling R-2 (“The inbox is amber.”): ")
        #expect(calls.args.isEmpty, "nothing is sent for a discussion: \(calls.args)")
    }
}
