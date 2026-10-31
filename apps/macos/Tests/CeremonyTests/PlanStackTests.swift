import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A ticket opened from the plan, as a navigation stack draws it (ov-873):
/// crumbs back to the plan, no tab strip, and Back returns to the plan.
@MainActor
@Suite(.serialized)
struct PlanStackTests {
    typealias Selection = ContentView.Selection
    typealias Crumb = WorkspaceNavigation.Crumb

    static let plan = Selection.workspace(host: "h", workspace: "w", focus: nil)
    static let title = "An already-paired browser starts after an update"
    static func task(_ id: String) -> Selection { .workspace(host: "h", workspace: "w", focus: .task(id)) }
    static let lane = Selection.workspace(host: "h", workspace: "w", focus: .plan(.lane("web-869")))

    /// What the outline gives a card: the workspace, its initiative, the card.
    static let outline = [
        Crumb(title: "Far Cooler", target: plan),
        Crumb(title: "Web client", target: .workspace(host: "h", workspace: "w", focus: .plan(.theme("web")))),
        Crumb(title: title, target: nil, glyph: "checklist"),
    ]
    static let origin = PlanStack.Origin(task: "ov-881", agent: "Builder 1", target: lane)

    @Test("Opened under an agent, the crumbs are Plan, the agent, the ticket, with no tab strip")
    func underAnAgent() {
        let page = PlanStack.page(base: Self.outline, place: Self.task("ov-881"), origin: Self.origin, besidePlan: true)
        #expect(page.crumbs.map(\.title) == ["Plan", "Builder 1", Self.title])
        #expect(page.crumbs.map(\.target) == [Self.plan, Self.lane, nil], "the first two go back; the ticket is where you are")
        #expect(!page.showsTabs, "no Overview, Agent, Changes strip over a page in a stack")
        #expect(!page.keepsMenus)
    }

    @Test("Opened through an initiative, the crumbs are Plan, the initiative, the ticket")
    func throughAnInitiative() {
        let page = PlanStack.page(base: Self.outline, place: Self.task("ov-881"), origin: nil, besidePlan: true)
        #expect(page.crumbs.map(\.title) == ["Plan", "Web client", Self.title])
        #expect(!page.showsTabs)
        #expect(page.keepsMenus, "the outline's menus still fit its own path")
    }

    @Test("An agent's crumb belongs to the ticket it opened, not the next one")
    func staleOrigin() {
        let page = PlanStack.page(base: Self.outline, place: Self.task("ov-882"), origin: Self.origin, besidePlan: true)
        #expect(page.crumbs.map(\.title) == ["Plan", "Web client", Self.title])
    }

    @Test("A window with no plan beside it keeps its jump bar and its tabs as they were")
    func noPlan() {
        let page = PlanStack.page(base: Self.outline, place: Self.task("ov-881"), origin: Self.origin, besidePlan: false)
        #expect(page.crumbs == Self.outline)
        #expect(page.showsTabs && page.keepsMenus)
    }

    @Test("Back from the ticket is the plan")
    func backReturnsToThePlan() {
        var history = NavigationHistory()
        history.record(from: Self.plan, to: Self.task("ov-881"))
        let back = history.goBack(from: Self.task("ov-881"), resolves: { _ in true })?.place
        #expect(back == Self.plan)
    }

    @Test("A ticket on an agent's card names the agent; the same ticket elsewhere does not")
    func cardsNameTheirAgent() async throws {
        let store = try await PlanColumnViewTests.store()
        let column = PlanColumn(plan: store.plan.plan)
        let agent = try #require(column.pipeline.flatMap(\.agents).first { !$0.tickets.isEmpty })
        var under: [String] = []
        var plain: [String] = []
        var actions = PlanColumnActions(openTicket: { plain.append($0) })
        actions.open(ticket: "ov-881", under: agent)
        #expect(plain == ["ov-881"] && under.isEmpty, "without the window's route it opens as before")
        actions.openTicketUnder = { agent, id in under.append("\(agent.name)/\(id)") }
        actions.open(ticket: "ov-881", under: agent)
        #expect(under == ["\(agent.name)/ov-881"])
    }
}
