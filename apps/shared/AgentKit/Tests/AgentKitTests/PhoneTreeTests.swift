import Foundation
import Testing

@testable import AgentKit

/// The One tree on a phone (ov-300): the phone's own fleet types mapped onto
/// `OneTree`'s input, the root it shows, what a tapped row does, where each
/// target goes on the phone's stack, and the orchestrator's state and line
/// for the strip. Worktrees and terminals are decoded from the fleet's JSON,
/// as the phone reads them.
struct PhoneTreeTests {
    typealias P = PlanModelTests
    typealias T = OneTreeTests

    static let billing = "ws-billing"
    static let repository = "repo-1"
    static let summary = WorkspaceSummary(
        id: billing, name: "Billing", taskPrefix: "bil", isMain: false, ordinal: 1, repository: repository)

    static func terminal(
        _ id: String, preset: String = "zsh", role: String? = nil, task: String? = nil, paneMode: String? = nil,
        activity: String? = nil, state: String = "running", extra: [String: Any] = [:]
    ) -> [String: Any] {
        var out: [String: Any] = ["id": id, "short": id, "title": "", "preset": preset, "state": state, "epoch": 1]
        out["role"] = role
        out["taskId"] = task
        out["paneMode"] = paneMode
        out["activity"] = activity
        return out.merging(extra) { _, new in new }
    }

    static func worktree(
        _ name: String, workspace: String? = billing, main: Bool = false, tasks: [String] = [],
        terminals: [[String: Any]] = [], repository: String = repository, state: String = "ready"
    ) -> [String: Any] {
        var out: [String: Any] = [
            "id": "wt-\(name)", "short": name, "repository": repository, "task": name, "branch": name,
            "worktree": "/repo/.claude/worktrees/\(name)", "state": state, "terminals": terminals,
            "isMainCheckout": main,
            "open_tasks": tasks.map { ["id": $0, "key": "ov-\($0.dropFirst())", "title": $0, "status": "in_progress"] },
        ]
        out["workspace"] = workspace
        return out
    }

    static func decode(_ worktrees: [[String: Any]]) throws -> [Worktree] {
        try JSONDecoder().decode([Worktree].self, from: JSONSerialization.data(withJSONObject: worktrees))
    }

    static func decodeTerminal(_ terminal: [String: Any]) throws -> Terminal {
        try JSONDecoder().decode(Terminal.self, from: JSONSerialization.data(withJSONObject: terminal))
    }

    static func row(_ id: String, _ status: TaskStatus = .inProgress) -> TaskRow {
        TaskRow(id: id, key: "ov-\(id.dropFirst())", title: "Title \(id)", status: status, statusSince: Date(timeIntervalSince1970: 0))
    }

    static func board(_ rows: [TaskRow]) -> TaskBoardModel {
        TaskBoardModel(columns: TaskBoardModel.order.map { status in
            TaskBoardColumn(status: status, rows: rows.filter { $0.status == status })
        })
    }

    /// Two themes, a lane in a worktree with an agent, a shell and a changes
    /// pane; another workspace's worktree; another workspace's worktree that
    /// holds one of these cards; the repository's checkout with a project
    /// shell, the orchestrator, a task's agent and a changes pane; and a
    /// loose worktree.
    static func fixture() throws -> (plan: PlanModel, board: TaskBoardModel, worktrees: [Worktree]) {
        let plan = try P.plan(
            themes: [T.theme("Plan", cards: ["t1", "t2"], ordinal: 0, ask: "Which one?"), T.theme("Mac", cards: ["t3"], ordinal: 1)],
            lanes: [T.lane("copy", "building", cards: ["t1", "t3"], path: ".claude/worktrees/copy")])
        let worktrees = try decode([
            worktree("copy", terminals: [
                terminal("c1", preset: "claude", role: "agent", task: "t1", activity: "working"),
                terminal("c2"),
                terminal("c3", paneMode: "changes"),
            ]),
            worktree("elsewhere", workspace: "ws-other"),
            worktree("borrowed", workspace: "ws-other", tasks: ["t2"]),
            worktree("overnight", workspace: nil, main: true, terminals: [
                terminal("m1"),
                terminal("m2", preset: "claude", role: "orchestrator"),
                terminal("m3", preset: "claude", role: "agent", task: "t1"),
                terminal("m4", paneMode: "changes"),
            ]),
            worktree("spike"),
        ])
        return (plan, board([row("t1"), row("t2", .inReview), row("t3"), row("t5", .todo)]), worktrees)
    }

    static func tree(filter: OneTreeFilter = .open, items: [NeedsYouItem] = []) throws -> OneTree {
        let f = try fixture()
        return OneTree.build(
            PhoneTree.input(
                summary: summary, board: f.board, plan: f.plan, worktrees: f.worktrees, pages: [], items: items,
                filter: filter, needsYouCount: 0))
    }

    // MARK: The input

    @Test("a lane's terminals are its worktree's panes, a changes pane left out")
    func laneTerminals() throws {
        let tree = try Self.tree()
        let lane = try #require(tree.allNodes.first { $0.target == .lane("lane-copy") })
        #expect(lane.children.map(\.target) == [
            .terminal(worktree: "wt-copy", terminal: "c1"), .terminal(worktree: "wt-copy", terminal: "c2"),
        ])
        #expect(lane.children.first?.detail == "Agent")
    }

    @Test("the workspace's worktrees, and another's only when it holds one of these cards")
    func whichWorktrees() throws {
        let f = try Self.fixture()
        let input = PhoneTree.input(
            summary: Self.summary, board: f.board, plan: f.plan, worktrees: f.worktrees, pages: [], items: [],
            filter: .open, needsYouCount: 0)
        #expect(input.worktrees.map(\.name) == ["copy", "borrowed", "spike"])
        // The borrowed one hangs under its card, the spike is loose.
        let tree = OneTree.build(input)
        let t2 = try #require(tree.allNodes.first { $0.target == .task("t2") })
        #expect(t2.children.map(\.target) == [.worktree("wt-borrowed")])
        #expect(tree.below.map(\.title) == ["Main Checkout", "Loose Worktrees"])
        #expect(tree.below[1].children.map(\.title) == ["spike"])
    }

    @Test("the checkout lists the project's own shells: not the orchestrator, a task's agent or a changes pane")
    func checkoutTerminals() throws {
        let tree = try Self.tree()
        let main = try #require(tree.below.first)
        #expect(main.children.map(\.target) == [.terminal(worktree: "wt-overnight", terminal: "m1")])
        #expect(main.detail == "1 shell")
    }

    @Test("a theme's ask and a card's item put dots on them, and the dot rolls up")
    func asks() throws {
        let item = NeedsYouItem(
            id: "decision:1", kind: .decision, rank: 1, since: nil, workspaceID: Self.billing,
            task: NeedsYouTask(id: "t3", key: "ov-3", title: "t3", status: "needs_decision"), question: "?")
        let other = NeedsYouItem(
            id: "decision:2", kind: .decision, rank: 1, since: nil, workspaceID: "ws-other",
            task: NeedsYouTask(id: "t5", key: "ov-5", title: "t5", status: "needs_decision"), question: "?")
        let tree = try Self.tree(items: [item, other])
        #expect(tree.tree[0].asks)
        let mac = tree.tree[1]
        #expect(!mac.asks && mac.holdsAsk && mac.showsDot(expanded: false))
        #expect(tree.allNodes.first { $0.target == .task("t3") }?.asks == true)
        // Another workspace's item asks nothing here.
        #expect(tree.allNodes.first { $0.target == .task("t5") }?.asks == false)
    }

    // MARK: The count

    @Test("the strip's count is the Mac's: this workspace's items and its themes' asks")
    func needsYouCount() throws {
        let f = try Self.fixture()
        let mine = NeedsYouItem(id: "ask:1", kind: .ask, rank: 1, since: nil, workspaceID: Self.billing, question: "?")
        let theirs = NeedsYouItem(id: "ask:2", kind: .ask, rank: 1, since: nil, workspaceID: "ws-other", question: "?")
        let read = PhoneTree.needsYouCount(
            summary: Self.summary, board: f.board, plan: f.plan, items: [mine, theirs], listRead: true, listServed: true)
        #expect(read == 2)  // one item, one theme asking
        // Before the list is read, the Needs Decision column counts beside it.
        let column = Self.board([Self.row("t9", .needsDecision)])
        let unread = PhoneTree.needsYouCount(
            summary: Self.summary, board: column, plan: f.plan, items: [mine], listRead: false, listServed: true)
        #expect(unread == 3)
    }

    // MARK: The root and taps

    @Test("the root leaves out the pinned places: Needs You is the app's root, and Plan is the sheet")
    func root() throws {
        let root = PhoneTree.root(try Self.tree())
        #expect(root.work.map(\.title) == ["Plan", "Mac", "No Theme"])
        #expect(root.below.map(\.title) == ["Main Checkout", "Loose Worktrees"])
        #expect(!(root.work + root.below).contains { $0.kind == .place })
    }

    @Test("a row with children pushes its level; a leaf opens what it points at; a subagent opens nothing")
    func taps() throws {
        let tree = try Self.tree()
        let theme = tree.tree[0]
        #expect(PhoneTree.tap(theme) == .push(theme.id))
        let shell = try #require(tree.allNodes.first { $0.target == .terminal(worktree: "wt-copy", terminal: "c2") })
        #expect(PhoneTree.tap(shell) == .open(.terminal(worktree: "wt-copy", terminal: "c2")))
        let leafCard = try #require(tree.allNodes.first { $0.target == .task("t5") })
        #expect(PhoneTree.tap(leafCard) == .open(.task("t5")))
        let subagent = OneTreeNode(id: "x/agent:a", kind: .subagent, title: "Builder", target: .orchestrator)
        #expect(PhoneTree.tap(subagent) == .none)
        let place = OneTreeNode(id: "place:plan", kind: .place, title: "Plan", target: .plan)
        #expect(PhoneTree.tap(place) == .none)
    }

    @Test("a level opens on its node's own page")
    func ownRows() throws {
        let tree = try Self.tree()
        #expect(PhoneTree.ownRow(tree.tree[0]) == "Theme Page")
        #expect(PhoneTree.ownRow(try #require(tree.allNodes.first { $0.target == .task("t1") })) == "Task Details")
        #expect(PhoneTree.ownRow(try #require(tree.allNodes.first { $0.target == .lane("lane-copy") })) == "Lane Page")
        #expect(PhoneTree.ownRow(try #require(tree.below.first)) == "Open Worktree")
        #expect(PhoneTree.ownRow(try #require(tree.tree.last)) == nil)  // No Theme
    }

    @Test("each target goes where the phone already opens it")
    func routes() {
        let place = PhoneWorkspace(runner: "r1", workspace: Self.billing)
        #expect(PhoneTree.route(.theme("th"), in: place) == .plan(place, page: .theme("th")))
        #expect(PhoneTree.route(.lane("l"), in: place) == .plan(place, page: .lane("l")))
        #expect(PhoneTree.route(.page("train"), in: place) == .plan(place, page: .page("train")))
        #expect(PhoneTree.route(.task("t1"), in: place) == .task(place, task: "t1"))
        #expect(PhoneTree.route(.worktree("w"), in: place) == .worktree(runner: "r1", worktree: "w", landing: .resume))
        #expect(
            PhoneTree.route(.terminal(worktree: "w", terminal: "x"), in: place)
                == .worktree(runner: "r1", worktree: "w", landing: .terminal("x")))
        for elsewhere: OneTreeTarget in [.orchestrator, .needsYou, .plan] {
            #expect(PhoneTree.route(elsewhere, in: place) == nil)
        }
    }

    @Test("a level is found by its node's id, and a gone node is nil")
    func findNode() throws {
        let tree = try Self.tree()
        let lane = try #require(tree.allNodes.first { $0.target == .lane("lane-copy") })
        #expect(PhoneTree.node(lane.id, in: tree) == lane)
        #expect(PhoneTree.node("theme:gone", in: tree) == nil)
    }

    // MARK: The orchestrator

    @Test("the orchestrator's state, from its pane")
    func orchestratorState() throws {
        func state(_ t: [String: Any]) throws -> PlanStripOrchestrator { PhoneTree.orchestrator(try Self.decodeTerminal(t)) }
        #expect(PhoneTree.orchestrator(nil) == .none)
        #expect(try state(Self.terminal("o", state: "starting")) == .starting)
        #expect(try state(Self.terminal("o", activity: "working", state: "lost")) == .stopped)
        #expect(try state(Self.terminal("o", activity: "blocked")) == .needsYou)
        #expect(try state(Self.terminal("o", activity: "working")) == .working)
        #expect(try state(Self.terminal("o", activity: "done")) == .done)
        #expect(try state(Self.terminal("o", activity: "done", extra: ["turnFailed": true])) == .failed)
        #expect(try state(Self.terminal("o", activity: "idle")) == .idle)
    }

    @Test("its line: the question it's blocked on, what it's doing, or what it last said; never just its headline")
    func orchestratorLine() throws {
        let blocked = try Self.decodeTerminal(
            Self.terminal("o", activity: "blocked", extra: ["blockedQuestion": "Run the migration?", "line": "Waiting"]))
        #expect(PhoneTree.line(blocked, state: .needsYou) == "Run the migration?")
        let working = try Self.decodeTerminal(
            Self.terminal("o", activity: "working", extra: ["line": "Dispatching ov-321", "said": "Earlier."]))
        #expect(PhoneTree.line(working, state: .working) == "Dispatching ov-321")
        let headline = try Self.decodeTerminal(
            Self.terminal("o", activity: "working", extra: ["line": "claude 4m", "headline": "claude 4m", "said": "Read the plan."]))
        #expect(PhoneTree.line(headline, state: .working) == "Read the plan.")
        let idle = try Self.decodeTerminal(Self.terminal("o", activity: "idle", extra: ["said": "All landed.", "line": "x"]))
        #expect(PhoneTree.line(idle, state: .idle) == "All landed.")
        #expect(PhoneTree.line(idle, state: .stopped) == nil)
    }

    // MARK: Segments and the stack

    @Test("a workspace with an orchestrator offers Orchestrator, Themes and Board; an implicit one Board and Worktrees")
    func segments() {
        #expect(WorkspaceSegment.offered(implicit: false) == [.orchestrator, .tree, .board])
        #expect(WorkspaceSegment.offered(implicit: true) == [.board, .worktrees])
        #expect(WorkspaceSegment.tree.title == "Themes")
    }

    @Test("a workspace remembered on Worktrees opens on Themes, its successor, and back")
    func successor() {
        #expect(WorkspaceSegment.shown(.worktrees, implicit: false) == .tree)
        #expect(WorkspaceSegment.shown(.tree, implicit: true) == .worktrees)
        #expect(WorkspaceSegment.shown(.orchestrator, implicit: true) == .board)
        #expect(WorkspaceSegment.shown(nil, implicit: false) == .orchestrator)
        let defaults = UserDefaults(suiteName: "phone-tree-tests")!
        let place = PhoneWorkspace(runner: "r1", workspace: Self.billing)
        defaults.set("worktrees", forKey: WorkspaceSegment.key(place))
        #expect(WorkspaceSegment.remembered(place, implicit: false, in: defaults) == .tree)
        defaults.removePersistentDomain(forName: "phone-tree-tests")
    }

    @Test("a tree level isn't a place a relaunch reopens; its workspace is")
    func relaunch() {
        let place = PhoneWorkspace(runner: "r1", workspace: Self.billing)
        let stack: [PhoneRoute] = [.workspace(place), .tree(place, node: "theme:x")]
        #expect(Destination(phoneStack: stack) == Destination(phoneStack: [.workspace(place)]))
        // And a worktree opened from a level takes its workspace from it.
        let deeper = stack + [.worktree(runner: "r1", worktree: "w", landing: .resume)]
        #expect(Destination(phoneStack: deeper)?.place == .worktree("w", workspace: Self.billing))
        // It survives being kept.
        #expect(PhoneLaunch.decode(PhoneLaunch.encode(stack)) == stack)
    }
}

extension PhoneTreeTests {
    @Test("a level not found waits for the board and the plan, and is gone only once both came back without it")
    func levelWaitsForItsReads() {
        #expect(PhoneTree.level(found: true, board: .pending, plan: .pending) == .node)
        #expect(PhoneTree.level(found: false, board: .pending, plan: .read) == .loading)
        #expect(PhoneTree.level(found: false, board: .read, plan: .pending) == .loading)
        #expect(PhoneTree.level(found: false, board: .read, plan: .failed) == .failed)
        #expect(PhoneTree.level(found: false, board: .failed, plan: .pending) == .failed)
        #expect(PhoneTree.level(found: false, board: .read, plan: .read) == .gone)
        #expect(PhoneTree.level(found: false, board: .read, plan: .notKept) == .gone)
    }
}
