import Foundation
import Testing

@testable import AgentKit

/// The overview's workspace level: which headings a runner's section is split
/// into, what is under each, and which pane is a heading's own row rather
/// than a card's tab. The iOS target has no unit tests, so what the grid
/// draws is decided here and read here.
struct ShellWorkspacesTests {
    /// Two repositories: `r1` split into Main and Billing, Billing's
    /// orchestrator running in Main's checkout, one worktree unclaimed; `r2`
    /// with Main alone.
    static let fleet = """
    {
      "runtime_healthy": true, "live_panes": 3,
      "workspaces": [
        {"id": "b1", "repository": "r1", "name": "Billing", "task_prefix": "bil",
         "is_main": false, "ordinal": 1, "orchestrator": "orch-b"},
        {"id": "m1", "repository": "r1", "name": "Main", "task_prefix": "ov",
         "is_main": true, "ordinal": 0, "orchestrator": "gone"},
        {"id": "m2", "repository": "r2", "name": "Main", "task_prefix": "sc",
         "is_main": true, "ordinal": 0}
      ],
      "worktrees": [
        {"id": "checkout", "short": "c", "repository": "r1", "task": "overnight", "branch": "main",
         "state": "active", "workspace": "m1",
         "terminals": [
           {"id": "shell", "short": "s", "title": "", "preset": "zsh", "state": "running",
            "epoch": 1, "workspace": "m1", "role": "shell"},
           {"id": "orch-b", "short": "o", "title": "", "preset": "claude", "state": "running",
            "epoch": 1, "workspace": "b1", "role": "orchestrator"}
         ]},
        {"id": "loose", "short": "l", "repository": "r1", "task": "loose", "branch": "l",
         "state": "active", "workspace": null, "terminals": []},
        {"id": "invoices", "short": "i", "repository": "r1", "task": "invoices", "branch": "i",
         "state": "active", "workspace": "b1", "terminals": []},
        {"id": "scratch", "short": "x", "repository": "r2", "task": "scratch", "branch": "x",
         "state": "active", "workspace": "m2", "terminals": []}
      ]
    }
    """

    /// Headings in drawing order: each repository's workspaces, Main first,
    /// then its Unclaimed; the worktrees under each; and a repository named on
    /// its headings because the runner has two.
    @Test func eachRepositorysWorkspacesThenItsUnclaimed() throws {
        let fleet = try FleetDecodeTests.decodeFleet(Self.fleet)
        let layout = try #require(fleet.shellLayout(names: ["r1": "overnight", "r2": "scratch"]))
        #expect(layout.headings.map(\.heading.name) == ["Main", "Billing", "Unclaimed", "Main"])
        #expect(layout.headings.map(\.heading.id) == ["m1", "b1", "unclaimed/r1", "m2"])
        #expect(layout.headings.map(\.worktrees) == [["checkout"], ["invoices"], ["loose"], ["scratch"]])
        #expect(layout.headings.map(\.heading.repository) == ["overnight", "overnight", "overnight", "scratch"])
        #expect(layout.headings[2].heading.isUnclaimed)
        // The shell's fleet is built in this order, so the bar walks the grid
        // in the order it is drawn.
        #expect(layout.order == ["checkout", "invoices", "loose", "scratch"])
        #expect(layout.headingOf["loose"] == "unclaimed/r1")
    }

    /// An orchestrator is its workspace's, found by its pane's role, and only
    /// when that pane is in the fleet: Main's names a terminal nobody listed,
    /// so Main has no orchestrator row rather than one that opens nothing.
    @Test func anOrchestratorIsItsWorkspacesRowWhereverItsPaneRuns() throws {
        let fleet = try FleetDecodeTests.decodeFleet(Self.fleet)
        let layout = try #require(fleet.shellLayout(names: [:]))
        #expect(layout.headings.map(\.orchestrator) == [nil, "orch-b", nil, nil])
        #expect(layout.orchestrators == ["orch-b"])
    }

    /// An orchestrator's tab is called after its workspace, so the bar in
    /// Main's checkout says the pane is Billing's manager and not one more
    /// of Main's terminals. Only a listed orchestrator gets a title.
    @Test func anOrchestratorsTabIsNamedForItsWorkspace() throws {
        let fleet = try FleetDecodeTests.decodeFleet(Self.fleet)
        let layout = try #require(fleet.shellLayout(names: [:]))
        #expect(layout.orchestratorTitles == ["orch-b": "Billing Orchestrator"])
    }

    /// One repository: nothing to tell the headings apart by, so none names it.
    @Test func oneRepositoryIsNotNamedOnEveryHeading() throws {
        let fleet = try FleetDecodeTests.decodeFleet(WorkspaceGroupsTests.twoWorkspaces)
        let layout = try #require(fleet.shellLayout(names: ["r1": "overnight"]))
        #expect(layout.headings.allSatisfy { $0.heading.repository == nil })
        // A workspace with no worktrees still has its heading.
        #expect(layout.headings.map(\.heading.name) == ["Main", "Billing", "Unclaimed"])
    }

    /// A runner without `workstreams` has no workspace level at all: the
    /// section it always had.
    @Test func aRunnerWithoutWorkspacesHasNoLayout() throws {
        let old = """
        {"runtime_healthy": true, "live_panes": 0,
         "worktrees": [{"id": "w1", "short": "w1", "repository": "r1", "task": "t",
                        "branch": "b", "state": "active", "terminals": []}]}
        """
        #expect(try FleetDecodeTests.decodeFleet(old).shellLayout(names: [:]) == nil)
    }

    // MARK: - Sections

    private static let runner = ShellRunnerLabel(id: "R", name: "laptop", keepsOrder: true)

    private static func card(_ id: String, under heading: String?, orchestrator: Bool = false)
        -> ShellWorktree
    {
        ShellWorktree(
            id: id, name: id, runner: "R", heading: heading,
            tabs: [
                ShellTab(id: "\(id)/diff", title: "Diff", mark: GlanceMark(attention: .quiet, core: nil)),
                ShellTab(
                    id: "\(id)/orch", title: "claude", mark: GlanceMark(attention: .quiet, core: nil),
                    isOrchestrator: orchestrator),
            ])
    }

    private static let headings = [
        ShellWorkspaceHeading(id: "m1", name: "Main"),
        ShellWorkspaceHeading(id: "b1", name: "Billing"),
        ShellWorkspaceHeading(id: "unclaimed/r1", name: "Unclaimed", isUnclaimed: true),
    ]

    private static let fleetOfCards = ShellFleet(worktrees: [
        card("checkout", under: "m1", orchestrator: true),
        card("a", under: "m1"),
        card("invoices", under: "b1"),
        card("loose", under: "unclaimed/r1"),
        card("b", under: "m1"),
    ])

    /// A section per heading, holding the cards under it; the runner's own
    /// heading on the first only.
    @Test func aRunnerWithWorkspacesIsASectionPerWorkspace() {
        let sections = Self.fleetOfCards.runnerSections(
            [Self.runner], headings: { _ in Self.headings })
        #expect(sections.map(\.heading?.name) == ["Main", "Billing", "Unclaimed"])
        #expect(sections.map { $0.cards.map(\.id) } == [["checkout", "a", "b"], ["invoices"], ["loose"]])
        #expect(sections.map(\.leadsRunner) == [true, false, false])
        #expect(Set(sections.map(\.id)).count == 3, "each section is its own collection")
    }

    /// The same fleet with no headings is today's one section.
    @Test func aRunnerWithoutHeadingsKeepsItsOneSection() {
        let sections = Self.fleetOfCards.runnerSections([Self.runner])
        #expect(sections.count == 1)
        #expect(sections[0].heading == nil)
        #expect(sections[0].id == "R")
        #expect(sections[0].cards.count == 5)
    }

    /// A drop inside a workspace reorders that workspace's cards and names
    /// nobody else's; a card carried into another workspace is not a move.
    @Test func aDropInAWorkspaceReordersOnlyThatWorkspace() throws {
        let sections = Self.fleetOfCards.runnerSections(
            [Self.runner], headings: { _ in Self.headings })
        let main = sections[0]
        let request = try #require(main.reorder(moving: ["b"], before: "checkout"))
        #expect(request.order == ["b", "checkout", "a"])
        #expect(main.reorder(moving: ["invoices"], before: "a") == nil)
    }

    /// A workspace with nothing in it keeps its heading, until a search: a
    /// search drops a section it emptied, and the runner's heading moves to
    /// the first section left.
    @Test func aSearchDropsTheWorkspacesItEmptied() {
        var fleet = Self.fleetOfCards
        fleet.worktrees.removeAll { $0.id == "invoices" }
        let all = fleet.runnerSections([Self.runner], headings: { _ in Self.headings })
        #expect(all.map(\.heading?.name) == ["Main", "Billing", "Unclaimed"])
        #expect(all[1].cards.isEmpty)

        let searched = fleet.runnerSections(
            [Self.runner], headings: { _ in Self.headings }, matching: "loose")
        #expect(searched.map(\.heading?.name) == ["Unclaimed"])
        #expect(searched.map(\.leadsRunner) == [true])
    }

    /// A card whose heading the layout does not name — the fleet read a
    /// moment before its layout — is drawn in a last section, not lost.
    @Test func aCardUnderNoKnownHeadingIsNotLost() {
        var fleet = Self.fleetOfCards
        fleet.worktrees.append(Self.card("new", under: "w-new"))
        let sections = fleet.runnerSections([Self.runner], headings: { _ in Self.headings })
        #expect(sections.last?.heading == nil)
        #expect(sections.last?.cards.map(\.id) == ["new"])
    }

    /// The orchestrator's pane is a tab of the worktree it runs in, and not
    /// on that worktree's card.
    @Test func anOrchestratorIsOffItsWorktreesCard() {
        let checkout = Self.card("checkout", under: "m1", orchestrator: true)
        #expect(checkout.tabs.count == 2)
        #expect(checkout.listedTabs.map(\.id) == ["checkout/diff"])
    }

    /// The card counts the tabs the bar shows, the orchestrator's included:
    /// open it and there are that many to swipe through.
    @Test func aCardCountsEveryTabTheBarShows() {
        var checkout = Self.card("checkout", under: "m1", orchestrator: true)
        #expect(checkout.cardSubtitle == "2 tabs")
        checkout.server = "eu-runner-1"
        #expect(checkout.cardSubtitle == "eu-runner-1 · 2 tabs")
        let diffOnly = ShellWorktree(
            id: "loose", name: "loose",
            tabs: [ShellTab(id: "loose/diff", title: "Diff", mark: GlanceMark(attention: .quiet, core: nil))])
        #expect(diffOnly.cardSubtitle == "1 tab")
    }
}
