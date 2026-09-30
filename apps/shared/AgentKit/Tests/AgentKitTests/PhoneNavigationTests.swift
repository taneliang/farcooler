import Foundation
import Testing

@testable import AgentKit

/// The iPhone's stack rules (ov-55 4A): where a launch opens, where a link
/// lands, what Needs You's Workspaces list holds, and what a refused answer
/// says. In AgentKit because the iOS target has no unit tests, and each of
/// these looks fine in a screenshot when it's wrong.
struct PhoneNavigationTests {
    // MARK: - Fixtures

    static let runner = "RUNNER-A"
    static let repository = "repo-1"
    static let main = "ws-main"
    static let billing = "ws-billing"

    static func terminal(
        _ id: String, activity: String? = nil, task: String? = nil, workspace: String? = nil,
        role: String? = nil
    ) -> Terminal {
        Terminal(
            id: id, short: id, title: "claude", preset: "claude", state: "running",
            activity: activity, epoch: 1, paneMode: "terminal", chatCapable: false,
            taskId: task, workspace: workspace, role: role)
    }

    static func worktree(
        _ id: String, workspace: String?, terminals: [Terminal], open: [String] = [],
        hidden: Bool = false
    ) -> Worktree {
        Worktree(
            id: id, short: id, repository: repository, task: id, branch: "b/\(id)",
            state: hidden ? "hidden" : "ready", terminals: terminals, workspace: workspace,
            openTasks: open.map { NeedsYouTask(id: $0, key: "bil-\($0)", title: $0, status: "in_progress") })
    }

    /// Main leads with an orchestrator in the checkout; Billing has a
    /// worktree with an agent on task t9 and a shell with no task; a
    /// scratch worktree is unclaimed, and an old one is hidden.
    static var fleet: Fleet {
        Fleet(
            runtimeHealthy: true, livePanes: 4,
            worktrees: [
                worktree(
                    "checkout", workspace: main,
                    terminals: [terminal("orch", workspace: main, role: "orchestrator")]),
                worktree(
                    "webhooks", workspace: billing,
                    terminals: [
                        terminal("agent", activity: "blocked", task: "t9", workspace: billing),
                        terminal("shell", workspace: billing),
                    ],
                    open: ["t9"]),
                worktree("scratch", workspace: nil, terminals: [terminal("loose")]),
                worktree("old", workspace: nil, terminals: [], hidden: true),
            ],
            workspaces: [
                WorkspaceSummary(
                    id: main, name: "Main", taskPrefix: "ove", isMain: true, ordinal: 0,
                    repository: repository, orchestrator: "orch"),
                WorkspaceSummary(
                    id: billing, name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1,
                    repository: repository),
            ])
    }

    static func item(_ id: String, workspace: String?, repository: String? = repository)
        -> NeedsYouItem
    {
        NeedsYouItem(
            id: id, kind: .blocked, rank: 1, since: nil, workspaceID: workspace,
            repositoryID: repository, question: "?")
    }

    static let place = PhoneWorkspace(runner: runner, workspace: billing)

    // MARK: - The launch

    /// **A launch that can't hear from every runner in ten seconds stays on
    /// Needs You**, rather than pushing the last workspace over whatever
    /// somebody has started reading a minute later.
    @Test("A launch still waiting on a runner gives up after ten seconds and stays")
    func aLaunchGivesUpAfterTenSeconds() {
        func decide(_ elapsed: TimeInterval) -> PhoneLaunch.Decision {
            PhoneLaunch.decide(
                [.read, .waiting], elapsed: elapsed, moved: false, linking: false, itemCount: 0,
                last: Self.place, exists: { _ in true })
        }
        #expect(decide(2) == .wait)
        #expect(decide(9.9) == .wait)
        #expect(decide(10) == .stay)
        #expect(decide(60) == .stay)
    }

    /// **Once every runner has said, a launch decides by ruling 4**, and a
    /// stack somebody moved, or a link landing, wins over it.
    @Test("A launch decides once every runner has answered, unless someone moved first")
    func aLaunchDecidesWhenEveryRunnerHasAnswered() {
        func decide(moved: Bool = false, linking: Bool = false, items: Int = 0)
            -> PhoneLaunch.Decision
        {
            PhoneLaunch.decide(
                [.read, .unreachable], elapsed: 1, moved: moved, linking: linking,
                itemCount: items, last: Self.place, exists: { _ in true })
        }
        #expect(decide() == .open([.workspace(Self.place)]))
        #expect(decide(items: 2) == .open([]))
        #expect(decide(moved: true) == .stay)
        #expect(decide(linking: true) == .stay)
    }

    // MARK: - Links

    /// **A link to an agent lands with its workspace and task under it.**
    @Test("A link to a task's agent pushes its workspace, then the task, then the pane")
    func aLinkToAnAgentPushesItsWorkspaceAndTask() {
        let link = Self.fleet.phoneLink(toTerminal: "agent", runner: Self.runner)
        #expect(
            link
                == PhoneLink(
                    stack: [
                        .workspace(Self.place), .task(Self.place, task: "t9"),
                        .worktree(runner: Self.runner, worktree: "webhooks", landing: .terminal("agent")),
                    ],
                    segment: nil))
    }

    /// A pane with no task and a worktree with one open task is shown under
    /// that task (`TaskLink`), so the link lands under it too.
    @Test("A link to a shell in a one-task worktree lands under that task")
    func aShellInAOneTaskWorktreeLandsUnderItsTask() {
        let link = Self.fleet.phoneLink(toTerminal: "shell", runner: Self.runner)
        #expect(link?.stack.count == 3)
        #expect(link?.stack[1] == .task(Self.place, task: "t9"))
    }

    /// **An orchestrator's link is its workspace's Orchestrator segment**,
    /// not a worktree: it leads the workspace and works no one task.
    @Test("A link to an orchestrator opens its workspace on the Orchestrator segment")
    func aLinkToAnOrchestratorOpensItsSegment() {
        let link = Self.fleet.phoneLink(toTerminal: "orch", runner: Self.runner)
        let main = PhoneWorkspace(runner: Self.runner, workspace: Self.main)
        #expect(link == PhoneLink(stack: [.workspace(main)], segment: .orchestrator))
    }

    /// An unclaimed worktree's pane has no workspace to put under it, and a
    /// terminal this runner doesn't have is no link at all.
    @Test("A link to an unclaimed pane is the pane alone, and to an unknown one is nothing")
    func anUnclaimedPaneIsThePaneAlone() {
        #expect(
            Self.fleet.phoneLink(toTerminal: "loose", runner: Self.runner)
                == PhoneLink(
                    stack: [.worktree(runner: Self.runner, worktree: "scratch", landing: .terminal("loose"))],
                    segment: nil))
        #expect(Self.fleet.phoneLink(toTerminal: "gone", runner: Self.runner) == nil)
    }

    // MARK: - The Workspaces list

    /// **Every workspace is a row with its own count, the unclaimed items
    /// counted apart, and a hidden worktree only under Hidden.**
    @Test("The Workspaces list counts each workspace's items and keeps hidden worktrees apart")
    func theWorkspacesListCountsAndKeepsHiddenApart() throws {
        let items = [
            Self.item("a", workspace: Self.billing), Self.item("b", workspace: Self.billing),
            Self.item("c", workspace: nil), Self.item("d", workspace: nil, repository: "other"),
        ]
        let sections = Self.fleet.phoneSections(runner: Self.runner, names: [:], items: items)
        let section = try #require(sections.first)
        #expect(sections.count == 1)
        #expect(section.workspaces.map(\.name) == ["Main", "Billing"])
        #expect(section.workspaces.map(\.count) == [0, 2])
        #expect(section.workspaces[0].orchestrator == "orch")
        #expect(section.workspaces[1].orchestrator == nil)
        #expect(section.unclaimedCount == 1, "only this repository's workspace-less item")
        #expect(section.unclaimed == ["scratch"], "the hidden worktree is not unclaimed")
        #expect(section.hidden == ["old"])
    }

    /// **On a runner without workspaces, an item names none, and counts
    /// under its repository's one implicit row**, which is where its
    /// worktrees are. There's no Unclaimed count to double it.
    @Test("On a runner without workspaces, an item counts under the repository's row")
    func anOlderRunnersItemCountsUnderTheImplicitRow() throws {
        var fleet = Self.fleet
        fleet.workspaces = nil
        let sections = fleet.phoneSections(
            runner: Self.runner, names: [Self.repository: "overnight"],
            items: [Self.item("a", workspace: nil)])
        let row = try #require(sections.first?.workspaces.first)
        #expect(sections.first?.workspaces.count == 1)
        #expect(row.isImplicit)
        #expect(row.name == "overnight")
        #expect(row.count == 1)
        #expect(sections.first?.unclaimedCount == 0)
    }

    // MARK: - Refusals and the caveat

    /// **Two refusals with one code get two sentences** (spec §2.5), keyed
    /// on the runner's `what`.
    @Test("A refused answer says whether someone got there first or it didn't arrive")
    func aRefusedAnswerSaysWhichItWas() {
        #expect(
            PhoneAnswer.refusal(word: "resource-conflict", what: "not_held", agent: "claude")
                == "Someone already answered this.")
        #expect(
            PhoneAnswer.refusal(word: "resource-conflict", what: "not_delivered", agent: "claude")
                == "Couldn’t reach claude. Try again.")
        #expect(
            PhoneAnswer.refusal(word: nil, what: nil, agent: "claude")
                == "Your answer may not have reached the runner. Try again.")
        let scoped = PhoneAnswer.refusal(word: "scope-denied", what: nil, agent: "claude")
        #expect(!scoped.isEmpty)
        #expect(!scoped.contains("scope-denied"), "a runner's word never reaches the screen")
    }

    /// **"Nothing needs you" is hedged by the runners that didn't answer.**
    @Test("Nothing needs you names the runners that aren't answering")
    func nothingNeedsYouNamesTheSilentRunners() {
        #expect(PhoneInbox.caveat(unanswered: []) == nil)
        #expect(
            PhoneInbox.caveat(unanswered: ["studio"])
                == "studio isn’t answering, so this may not be everything.")
        #expect(
            PhoneInbox.caveat(unanswered: ["studio", "gpu-box-2"])
                == "2 runners aren’t answering, so this may not be everything.")
    }

    // MARK: - While a list is on its way

    /// **A runner whose list hasn't been read shows its blocked agents**, as
    /// the widget counts them for it, so the app and the glances agree while
    /// the read is out. A runner that has a list shows that and only that.
    @Test("An unread runner's blocked agents are shown, as the glances count them")
    func anUnreadRunnersBlockedAgentsAreShown() {
        let answered = [Self.item("decision:1", workspace: Self.billing)]
        let shown = PhoneInbox.shown(
            lists: ["A": answered], unread: ["B": Self.fleet.olderPanes()])
        #expect(shown.count == 2)
        #expect(shown.filter { $0.runner == "A" }.map(\.itemID) == ["decision:1"])
        let derived = shown.filter { $0.runner == "B" }
        #expect(derived.map(\.itemID) == ["blocked:agent"])
        #expect(derived.allSatisfy { $0.isDerived })
        // A runner with a list is never derived over, even from its fleet.
        #expect(
            PhoneInbox.shown(lists: ["A": answered], unread: ["A": Self.fleet.olderPanes()])
                .count == 1)
    }
}
