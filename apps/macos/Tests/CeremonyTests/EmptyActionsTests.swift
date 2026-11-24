import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Every empty state says what happens next and offers it (ov-519): with no
/// orchestrator, Start Orchestrator, checked against what's installed; with
/// one, the next thing to tell it.
@MainActor
@Suite(.serialized)
struct EmptyActionsTests {
    private static func invite(
        seated: Bool = false, canAct: Bool = true, agents: [String]? = nil, place: RunnerPlace = .thisMac
    ) -> OrchestratorInvite {
        OrchestratorInvite(
            seated: seated, canAct: canAct, availability: HarnessAvailability(agentsFound: agents), place: place,
            onStart: { _ in })
    }

    // MARK: - Which agents start

    @Test("Start Orchestrator offers the agents that are installed, and says which aren't")
    func choices() {
        #expect(OrchestratorChoices(HarnessAvailability(agentsFound: nil)) == .all)
        #expect(OrchestratorChoices(HarnessAvailability(agentsFound: ["claude", "codex", "cursor-agent"])) == .all)
        #expect(
            OrchestratorChoices(HarnessAvailability(agentsFound: ["codex"]))
                == .some(installed: [.codex], missing: [.claude, .cursor]))
        #expect(OrchestratorChoices(HarnessAvailability(agentsFound: [])) == .none)

        let some = OrchestratorChoices(HarnessAvailability(agentsFound: ["claude"]))
        #expect(some.sentence(on: .thisMac) == "Codex and Cursor aren’t installed on this Mac.")
        #expect(
            OrchestratorChoices.none.sentence(on: .runner("studio"))
                == "No agent is installed on studio. Install Claude Code, Codex, or the Cursor CLI first.")
        #expect(OrchestratorChoices.all.sentence(on: .thisMac) == nil)
    }

    @Test("The column offers only an installed agent to start, so a missing one never starts")
    func offersFollowWhatIsInstalled() {
        let codexOnly = OrchestratorChoices(HarnessAvailability(agentsFound: ["codex"]))
        #expect(ConversationColumn.offers(.none, choices: codexOnly) == [.start(.codex)])
        #expect(ConversationColumn.offers(.none, choices: .none).isEmpty)
        #expect(ConversationColumn.offers(.none) == OrchestratorHarness.allCases.map(ConversationColumn.Offer.start))
        // Restart and Replace are about an orchestrator that exists.
        #expect(ConversationColumn.offers(.lost, choices: .none) == [.restart, .replace])
    }

    // MARK: - The conversation column

    private static func column(_ invite: OrchestratorInvite?) async -> EmptyStateProbe.Drawn {
        await EmptyStateProbe.draw(
            ConversationPlaceholder(state: .none, offers: [], onRestart: {}, onReplace: {})
                .environment(\.orchestratorInvite, invite))
    }

    @Test("With no orchestrator the column invites, with Start Orchestrator")
    func columnInvites() async {
        let drawn = await Self.column(Self.invite())
        #expect(drawn.contains("start-orchestrator"), "\(drawn.ids)")
        #expect(!drawn.contains("orchestrator-missing"), "all installed says nothing about missing")
    }

    @Test("A runner that doesn't let this window act offers no start")
    func readOnlyRunner() async {
        let drawn = await Self.column(Self.invite(canAct: false))
        #expect(!drawn.contains("start-orchestrator"), "\(drawn.ids)")
    }

    @Test("With no agent installed the column says which to install, with Check Again")
    func noAgentInstalled() async {
        let drawn = await Self.column(Self.invite(agents: []))
        #expect(drawn.contains("orchestrator-missing") && drawn.contains("orchestrator-check-again"), "\(drawn.ids)")
    }

    @Test("With one agent missing it says so, and Check Again waits until none is installed")
    func someMissing() async {
        let drawn = await Self.column(Self.invite(agents: ["claude"]))
        #expect(drawn.contains("start-orchestrator") && drawn.contains("orchestrator-missing"), "\(drawn.ids)")
        #expect(!drawn.contains("orchestrator-check-again"))
    }

    // MARK: - The Plan

    private static func plan(_ invite: OrchestratorInvite?) async -> EmptyStateProbe.Drawn {
        await EmptyStateProbe.draw(PlanEmptyNotice().environment(\.orchestratorInvite, invite))
    }

    @Test("An empty Plan with no orchestrator offers Start Orchestrator")
    func planOffersStart() async throws {
        let calls = PlanViewTests.Calls()
        calls.plan = Data(#"{"now_ms":0,"themes":[],"lines":[],"cards":[],"unthemed_open":0}"#.utf8)
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        let frames = await PlanThemePageTests.frames(
            PlanOverviewView(plan: store.plan, statuses: [:], selected: nil, onOpen: { _ in })
                .environment(\.orchestratorInvite, Self.invite()),
            width: 600, height: 800)
        #expect(frames["start-orchestrator"] != nil, "\(frames.keys.sorted())")
        #expect(frames["plan-unthemed-count"] == nil)
        let column = await PlanThemePageTests.frames(
            PlanHome(board: store, needsYou: PlanNeedsYou(), onOpen: { _ in }), width: 600)
        #expect(column["plan-column"] != nil)
        #expect(column["plan-unthemed-count"] == nil)
    }

    @Test("The Plan overview and column show a positive count of open cards without a theme")
    func planShowsUnthemedCount() async throws {
        let calls = PlanViewTests.Calls()
        calls.plan = Data(#"""
            {"now_ms":0,"themes":[],"lines":[],"unthemed_open":1,
             "cards":[{"task":"00000000-0000-0000-0000-000000001002","key":"ov-2","title":"Title of ov-2","status":"backlog"}]}
            """#.utf8)
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        let overview = await PlanThemePageTests.frames(
            PlanOverviewView(plan: store.plan, statuses: store.board.statuses, selected: nil, onOpen: { _ in }),
            width: 600)
        #expect(overview["plan-unthemed-count"] != nil, "\(overview.keys.sorted())")
        let column = await PlanThemePageTests.frames(
            PlanHome(board: store, needsYou: PlanNeedsYou(), onOpen: { _ in }), width: 600)
        #expect(column["plan-unthemed-count"] != nil, "\(column.keys.sorted())")
    }

    @Test("Beside the conversation column the Plan keeps its sentence and leaves the button to the column")
    func planBesideTheColumn() async {
        for invite in [Self.invite(), Self.invite(seated: true)] {
            let drawn = await EmptyStateProbe.draw(
                PlanEmptyNotice().environment(\.orchestratorInvite, invite).environment(\.besideOrchestratorColumn, true))
            #expect(!drawn.contains("start-orchestrator"), "\(drawn.ids)")
        }
    }

    @Test("With an orchestrator the Plan says what to tell it, and offers no start")
    func planWithOrchestrator() async {
        let drawn = await Self.plan(Self.invite(seated: true))
        #expect(!drawn.contains("start-orchestrator"), "\(drawn.ids)")
    }

    @Test("With none to start, as on a runner without workspaces, the Plan keeps its old words")
    func planWithoutWorkspace() async {
        let drawn = await Self.plan(nil)
        #expect(!drawn.contains("start-orchestrator"), "\(drawn.ids)")
    }

    // MARK: - Tickets

    private static func overview(_ invite: OrchestratorInvite?) async throws -> [String: CGRect] {
        let store = try await PlanSectionViewTests.store(.quiet)
        return await PlanThemePageTests.frames(
            PlanOverviewView(
                plan: store.plan, statuses: store.board.statuses, selected: nil, needsYou: 0, onOpen: { _ in },
                defaults: PlanViewTests.defaults()
            ).environment(\.orchestratorInvite, invite),
            width: 600, height: 3200)
    }

    @Test("A queue with nothing in it says how work gets in: Show Orchestrator with one, Start without")
    func emptyQueueOffersTheStep() async throws {
        let seated = try await Self.overview(Self.invite(seated: true))
        #expect(seated["plan-waiting-empty"] != nil && seated["plan-next-step"] != nil, "\(seated.keys.sorted())")
        let step = try #require(seated["plan-next-step"])
        let line = try #require(seated["plan-waiting-empty"])
        #expect(step.minY >= line.maxY - 0.5, "the step is under what it follows: \(line) \(step)")
        let unseated = try await Self.overview(Self.invite())
        #expect(unseated["plan-next-step"] != nil && unseated["start-orchestrator"] != nil, "\(unseated.keys.sorted())")
        let none = try await Self.overview(nil)
        #expect(none["plan-waiting-empty"] != nil && none["plan-next-step"] == nil, "\(none.keys.sorted())")
        let readOnly = try await Self.overview(Self.invite(canAct: false))
        #expect(readOnly["plan-next-step"] == nil)
    }

    @Test("Beside the column the empty queue says how work gets in and draws no button")
    func queueBesideTheColumn() async throws {
        let store = try await PlanSectionViewTests.store(.quiet)
        for invite in [Self.invite(), Self.invite(seated: true)] {
            let drawn = await PlanThemePageTests.frames(
                PlanOverviewView(
                    plan: store.plan, statuses: store.board.statuses, selected: nil, needsYou: 0, onOpen: { _ in },
                    defaults: PlanViewTests.defaults()
                ).environment(\.orchestratorInvite, invite).environment(\.besideOrchestratorColumn, true),
                width: 600, height: 3200)
            #expect(drawn["plan-next-step"] != nil && drawn["start-orchestrator"] == nil, "\(drawn.keys.sorted())")
        }
    }

    // MARK: - Needs You

    private static func needsYou(_ invite: OrchestratorInvite?) async -> EmptyStateProbe.Drawn {
        await EmptyStateProbe.draw(
            NeedsYouView(
                items: [], olderRunners: [], canAct: { _ in false }, onOpen: { _ in },
                onAnswerAsk: { _, _ in nil }, onDecide: { _, _ in false })
                .environment(\.orchestratorInvite, invite))
    }

    @Test("An empty Needs You with no orchestrator running says so and offers Start Orchestrator, not Show")
    func needsYouOffersStart() async {
        let drawn = await Self.needsYou(Self.invite())
        #expect(drawn.contains("needs-you-no-agents") && drawn.contains("start-orchestrator"), "\(drawn.ids)")
        // There is nothing to show: no button of that name.
        #expect(!drawn.contains("needs-you-show-orchestrator"))
    }

    @Test("Needs You checks the agent is installed too, and a read-only runner offers no start")
    func needsYouChecksInstalled() async {
        let none = await Self.needsYou(Self.invite(agents: []))
        #expect(none.contains("orchestrator-missing") && none.contains("orchestrator-check-again"), "\(none.ids)")
        let readOnly = await Self.needsYou(Self.invite(canAct: false))
        #expect(!readOnly.contains("start-orchestrator"), "\(readOnly.ids)")
    }

    @Test("With an orchestrator running, an empty Needs You is just the page")
    func needsYouQuiet() async {
        for invite in [Self.invite(seated: true), nil] {
            let drawn = await Self.needsYou(invite)
            #expect(!drawn.contains("start-orchestrator") && !drawn.contains("needs-you-no-agents"), "\(drawn.ids)")
        }
    }

    // MARK: - The Tasks list

    private static func tasks(_ invite: OrchestratorInvite?) async -> EmptyStateProbe.Drawn {
        await EmptyStateProbe.draw(BoardBlankState().environment(\.orchestratorInvite, invite), width: 300)
    }

    @Test("The empty Tasks list with no orchestrator offers to start one")
    func tasksOfferStart() async {
        let drawn = await Self.tasks(Self.invite())
        #expect(drawn.contains("board-empty") && drawn.contains("start-orchestrator"), "\(drawn.ids)")
        // Narrow as the sidebar is, the button stays inside it.
        let frame = drawn.frame("start-orchestrator")
        #expect(frame.map { $0.minX >= 0 && $0.maxX <= 300 } == true, "\(String(describing: frame))")
    }

    @Test("With an orchestrator the empty Tasks list offers nothing to start")
    func tasksWithOrchestrator() async {
        let drawn = await Self.tasks(Self.invite(seated: true))
        #expect(drawn.contains("board-empty") && !drawn.contains("start-orchestrator"), "\(drawn.ids)")
    }
}
