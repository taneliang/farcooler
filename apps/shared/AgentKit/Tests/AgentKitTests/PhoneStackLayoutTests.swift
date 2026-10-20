import Foundation
import Testing

@testable import AgentKit

/// The iPhone's Plan as a sheet (ov-616): the Plan sheet's status filters on
/// the One tree, and where each screen of a kept or linked stack opens, on
/// the stack, inside the sheet, or covering both.
struct PhoneStackLayoutTests {
    typealias Tree = PhoneTreeTests

    static func tree(_ narrow: PlanFilter?, items: [NeedsYouItem] = []) throws -> OneTree {
        let f = try Tree.fixture()
        return OneTree.build(
            PhoneTree.input(
                summary: Tree.summary, board: f.board, plan: f.plan, worktrees: f.worktrees, pages: [], items: items,
                filter: .open, narrow: narrow, needsYouCount: 0))
    }

    static func tasks(_ tree: OneTree) -> Set<String> {
        Set(tree.allNodes.compactMap { node in
            if case .task(let id)? = node.target { return id }
            return nil
        })
    }

    // MARK: Filters

    @Test("The four filters, in the sheet's order and words")
    func titles() {
        #expect(PlanFilter.allCases.map(\.title) == ["Needs You", "In Progress", "In Review", "Done"])
    }

    @Test("In Progress and In Review list only their cards, and a theme with none of them goes")
    func statusFilters() throws {
        #expect(Self.tasks(try Self.tree(.inProgress)) == ["t1", "t3"])
        let review = try Self.tree(.inReview)
        #expect(Self.tasks(review) == ["t2"])
        // Mac (t3, in progress) has nothing in review.
        #expect(!review.tree.contains { $0.title == "Mac" })
    }

    @Test("Done reaches finished cards, which Not Done only folds")
    func doneFilter() throws {
        let f = try Tree.fixture()
        var board = f.board
        board = Tree.board(board.rows + [Tree.row("t6", .done)])
        let done = OneTree.build(
            PhoneTree.input(
                summary: Tree.summary, board: board, plan: f.plan, worktrees: f.worktrees, pages: [], items: [],
                filter: .open, narrow: .done, needsYouCount: 0))
        #expect(Self.tasks(done) == ["t6"])
        #expect(!done.allNodes.contains { $0.kind == .doneFold }, "nothing folds under Done")
    }

    @Test("Needs You lists the cards that ask, and a theme that asks itself")
    func needsYouFilter() throws {
        let item = NeedsYouItem(
            id: "decision:1", kind: .decision, rank: 1, since: nil, workspaceID: Tree.billing,
            task: NeedsYouTask(id: "t3", key: "ov-3", title: "t3", status: "needs_decision"), question: "?")
        let tree = try Self.tree(.needsYou, items: [item])
        #expect(Self.tasks(tree) == ["t3"])
        // Plan has no card that asks, but asks itself ("Which one?").
        #expect(tree.tree.map(\.title).contains("Plan"))
    }

    @Test("No filter is Not Done, as before")
    func noFilter() throws {
        #expect(Self.tasks(try Self.tree(nil)) == Self.tasks(try Tree.tree()))
    }

    @Test("A workspace's filter is kept, and cleared")
    func remembered() throws {
        let defaults = try #require(ScratchDefaults.suite("plan-filter-\(UUID())"))
        #expect(PlanFilter.remembered(runner: "r", workspace: "w", in: defaults) == nil)
        PlanFilter.remember(.inReview, runner: "r", workspace: "w", in: defaults)
        #expect(PlanFilter.remembered(runner: "r", workspace: "w", in: defaults) == .inReview)
        PlanFilter.remember(nil, runner: "r", workspace: "w", in: defaults)
        #expect(PlanFilter.remembered(runner: "r", workspace: "w", in: defaults) == nil)
    }

    // MARK: Where a stack's screens open

    static let place = PhoneWorkspace(runner: "R", workspace: "W")
    static let other = PhoneWorkspace(runner: "R", workspace: "X")
    static let pane = PhoneRoute.worktree(runner: "R", worktree: "wt", landing: .resume)

    static func layout(_ stack: [PhoneRoute], sheets: Bool = true) -> PhoneStackLayout {
        PhoneStackLayout.of(stack, sheets: { _ in sheets })
    }

    @Test("A task, a history, a plan page and a tree level open inside the Plan sheet over their workspace")
    func insideTheSheet() {
        let task = PhoneRoute.task(Self.place, task: "t1")
        let level = PhoneRoute.tree(Self.place, node: "theme:a")
        let layout = Self.layout([.workspace(Self.place), level, task])
        #expect(layout.path == [.workspace(Self.place)])
        #expect(layout.sheet == PhonePlanSheet(place: Self.place, path: [level, task]))
        #expect(layout.cover == nil)
        #expect(layout.stack == [.workspace(Self.place), level, task], "kept as it was linked")
        #expect(Self.layout([.workspace(Self.place), .history(Self.place, status: "done")]).sheet?.path.count == 1)
        #expect(Self.layout([.workspace(Self.place), .plan(Self.place, page: .theme("a"))]).sheet != nil)
    }

    @Test("A workspace alone opens no sheet")
    func workspaceAlone() {
        let layout = Self.layout([.workspace(Self.place)])
        #expect(layout.path == [.workspace(Self.place)] && layout.sheet == nil && layout.cover == nil)
        #expect(Self.layout([]) == PhoneStackLayout(path: [], sheet: nil, cover: nil))
    }

    @Test("A worktree covers its workspace, and a task under it goes, so Back is the workspace and the list is two steps away")
    func worktreeOnTop() {
        let layout = Self.layout([.workspace(Self.place), .task(Self.place, task: "t1"), Self.pane])
        #expect(layout.path == [.workspace(Self.place)])
        #expect(layout.sheet == nil)
        #expect(layout.cover == Self.pane)
        #expect(Self.layout([Self.pane]).path.isEmpty)
    }

    @Test("A workspace whose screen is the Plan itself pushes its screens, as an iPad does")
    func pushedWhereThereIsNoSheet() {
        let task = PhoneRoute.task(Self.place, task: "t1")
        let layout = Self.layout([.workspace(Self.place), task], sheets: false)
        #expect(layout.path == [.workspace(Self.place), task])
        #expect(layout.sheet == nil)
        // Under a worktree they stay: that workspace's stack is pushed.
        #expect(Self.layout([.workspace(Self.place), task, Self.pane], sheets: false).path == [.workspace(Self.place), task])
    }

    @Test("Only the top workspace's screens go in its sheet; another workspace's stay where they are")
    func onlyItsOwn() {
        let theirs = PhoneRoute.task(Self.other, task: "t9")
        let layout = Self.layout([.workspace(Self.other), theirs, .workspace(Self.place)])
        #expect(layout.path == [.workspace(Self.other), theirs, .workspace(Self.place)])
        #expect(layout.sheet == nil)
    }

    @Test("All Agents is a screen on the stack, which a relaunch doesn't reopen")
    func allAgents() {
        #expect(!PhoneRoute.agents.opensInPlan)
        #expect(Self.layout([.agents]).path == [.agents])
        #expect(Destination(phoneStack: [.agents]) == nil)
        #expect(PhoneLaunch.decode(PhoneLaunch.encode([.agents])) == [.agents])
    }
}
