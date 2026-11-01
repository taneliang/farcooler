import Foundation
import Testing

@testable import AgentKit

/// One tree (ov-321): the sidebar's outline, Theme › Task › Ticket ›
/// Terminals, built from the plan as the CLI writes it and the board's
/// cards. Every plan here goes through `PlanModel.decode`, the parser the
/// app reads `plan --json` with.
struct OneTreeTests {
    typealias P = PlanModelTests

    static var root: URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return root
    }

    /// `test/fixtures/plan-seeded.json`: a real board's plan, from the CLI.
    static func seeded() throws -> PlanModel {
        struct Seeded: Decodable { var plan: PlanModel }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/plan-seeded.json"))
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(Seeded.self, from: data).plan
    }

    static func task(_ id: String, _ status: TaskStatus = .inProgress, at: Int64 = 0, worktree: String? = nil)
        -> OneTreeTask
    {
        OneTreeTask(id: id, key: "ov-\(id.dropFirst())", title: "Title \(id)", status: status, activityMs: at, worktreeID: worktree)
    }

    static func worktree(_ name: String, terminals: [OneTreeTerminal] = [], tasks: [String] = []) -> OneTreeWorktree {
        OneTreeWorktree(
            id: "wt-\(name)", name: name, path: "/repo/.claude/worktrees/\(name)", branch: name, terminals: terminals,
            taskIDs: tasks)
    }

    /// A ticket as `plan --json` writes it, with a worktree path and branch.
    static func line(
        _ task: String, _ stage: String, branch: String = "", path: String = "", fixing: Bool = false,
        since: Int64 = P.now - 600_000
    ) -> [String: Any] {
        var line = P.line(task, stage, since: since, fixing: fixing, title: "Title \(task)")
        line["worktree_path"] = path
        line["branch"] = branch
        return line
    }

    static func theme(_ name: String, cards: [String], ordinal: Int, ask: String = "", state: String = "active", storyAt: Int64 = 0)
        -> [String: Any]
    {
        var theme = P.theme(name, cards: cards, ordinal: ordinal, state: state)
        theme["owner_ask"] = ask
        theme["story_at"] = storyAt
        return theme
    }

    /// The board the captures are seeded with, in miniature: two themes, a
    /// ticket in a fix round (t1), two tickets sharing a worktree and a branch
    /// (copy: t2 and t3), and a loose worktree.
    static func board(asks: OneTreeAsks = OneTreeAsks(), filter: OneTreeFilter = .open) throws -> OneTreeInput {
        let plan = try P.plan(
            themes: [
                theme("Plan", cards: ["t1", "t2", "t9"], ordinal: 0),
                theme("Mac", cards: ["t3", "t4"], ordinal: 1, storyAt: P.now),
            ],
            lines: [
                line("t1", "building", branch: "primary", path: ".claude/worktrees/primary", fixing: true),
                line("t2", "building", branch: "copy", path: ".claude/worktrees/copy"),
                line("t3", "building", branch: "copy", path: ".claude/worktrees/copy"),
            ],
            agents: [P.agent("a1", tickets: [(task: "t1", role: "fix")])])
        return OneTreeInput(
            tasks: [
                task("t1", at: 10), task("t2", .inReview), task("t3"), task("t4", .needsDecision), task("t9", .done),
                task("t5", .todo), task("t6", .backlog),
            ],
            plan: plan,
            worktrees: [
                worktree("primary"),
                worktree("copy", terminals: [
                    OneTreeTerminal(id: "c1", title: "claude", isAgent: true), OneTreeTerminal(id: "c2", title: "zsh", isAgent: false),
                ]),
                worktree("spike"),
            ],
            mainCheckout: OneTreeWorktree(
                id: "main", name: "overnight", isMainCheckout: true,
                terminals: [OneTreeTerminal(id: "m1", title: "zsh", isAgent: false), OneTreeTerminal(id: "m2", title: "logs", isAgent: false)]),
            pages: [OneTreePage(slot: "landing", title: "Landing"), OneTreePage(slot: "copy", title: "Copy rules", themeID: "theme-Mac")],
            asks: asks, filter: filter, needsYouCount: 2)
    }

    static func node(_ tree: OneTree, _ id: String) -> OneTreeNode? { tree.allNodes.first { $0.id == id } }

    /// The board with every ticket landed: no work moving, so no theme opens by default.
    static func quietBoard(asks: OneTreeAsks = OneTreeAsks()) throws -> OneTreeInput {
        var input = try board(asks: asks)
        for index in input.plan.lines.indices { input.plan.lines[index].stage = .landed }
        return input
    }

    // MARK: The shape

    @Test("Top to bottom: three places, the themes in plan order, No Theme, then Main Checkout and Loose Worktrees")
    func theShape() throws {
        let tree = OneTree.build(try Self.board())
        #expect(tree.places.map(\.title) == ["Needs You", "Plan"])
        #expect(tree.tree.map(\.title) == ["Plan", "Mac", "No Theme"])
        #expect(tree.below.map(\.title) == ["Main Checkout", "Loose Worktrees"])
        #expect(tree.places[0].detail == "2")
    }

    @Test("Under a theme: its open cards in the theme's order, then its done ones folded, then its pages")
    func underATheme() throws {
        let tree = OneTree.build(try Self.board())
        let plan = try #require(tree.tree.first)
        #expect(plan.children.map(\.kind) == [.task, .task, .doneFold])
        #expect(plan.children.map(\.key) == ["ov-1", "ov-2", ""])
        #expect(plan.children[2].title == "1 done")
        #expect(plan.children[2].children.map(\.key) == ["ov-9"])
        let mac = tree.tree[1]
        #expect(mac.children.last?.kind == .page)
        #expect(mac.children.last?.title == "Copy rules")
        #expect(mac.detail == "0/0")
    }

    @Test("Two tickets on one worktree each show its panes, under their own cards, as one target apiece")
    func ticketsSharingAWorktree() throws {
        let tree = OneTree.build(try Self.board())
        let t2 = tree.allNodes.filter { $0.target == .ticket("t2") }
        let t3 = tree.allNodes.filter { $0.target == .ticket("t3") }
        #expect(t2.count == 1 && t3.count == 1, "a ticket is one row, under its one card")
        #expect(t2[0].id == "theme:theme-Plan/task:t2/ticket:t2" && t3[0].id == "theme:theme-Mac/task:t3/ticket:t3")
        #expect(t2[0].children.map(\.title) == ["claude", "zsh"] && t3[0].children.map(\.title) == ["claude", "zsh"])
        #expect(t2[0].children.map(\.target) == t3[0].children.map(\.target), "the same panes")
        #expect(t2[0].also.isEmpty && t3[0].also.isEmpty, "a ticket works one card")
    }

    @Test("A card has one ticket, and a fix round is that ticket saying Fixing")
    func aFixRound() throws {
        let tree = OneTree.build(try Self.board())
        let t1 = try #require(Self.node(tree, "theme:theme-Plan/task:t1"))
        #expect(t1.children.map(\.title) == ["ov-1"])
        #expect(t1.children.map(\.detail) == ["Fixing"])
        #expect(t1.children.map(\.ticketStage) == [.building], "the stage stays Building; Fixing is its open work")
        let t2 = try #require(Self.node(tree, "theme:theme-Plan/task:t2"))
        #expect(t2.children.map(\.detail) == ["Building"])
    }

    @Test("A ticket is its worktree: its terminals are the worktree's, joined on the path the plan records")
    func aTicketIsItsWorktree() throws {
        let tree = OneTree.build(try Self.board())
        let copy = try #require(Self.node(tree, "theme:theme-Plan/task:t2/ticket:t2"))
        #expect(copy.children.map(\.title) == ["claude", "zsh"])
        // An agent says so; a shell says nothing beside its name (review L6).
        #expect(copy.children.map(\.detail) == ["Agent", ""])
        #expect(copy.children[0].target == .terminal(worktree: "wt-copy", terminal: "c1"))
    }

    @Test("The join: the exact path, then a path recorded relative to the repository, then the branch")
    func theJoin() throws {
        let worktrees = [
            OneTreeWorktree(id: "a", name: "a", path: "/r/.claude/worktrees/x", branch: "bx"),
            OneTreeWorktree(id: "b", name: "b", path: ".claude/worktrees/x", branch: "by"),
            OneTreeWorktree(id: "c", name: "c", path: "/r/elsewhere", branch: "feat/z"),
        ]
        let plan = try P.plan(themes: [], lines: [
            Self.line("t1", "building", path: ".claude/worktrees/x"),
            Self.line("t2", "building", path: "/.claude/worktrees/x/"),
            Self.line("t3", "building", path: ".claude/worktrees/q"),
        ])
        var branch = plan.lines[2]
        branch.branch = "feat/z"
        #expect(OneTreeBuilder.worktree(of: plan.lines[0], in: worktrees)?.id == "b")
        #expect(OneTreeBuilder.worktree(of: plan.lines[1], in: worktrees)?.id == "a")
        #expect(OneTreeBuilder.worktree(of: plan.lines[2], in: worktrees) == nil)
        #expect(OneTreeBuilder.worktree(of: branch, in: worktrees)?.id == "c")
    }

    @Test("Subagents: the plan's agents on a ticket that have no pane, each saying it has no terminal")
    func subagents() throws {
        let tree = OneTree.build(try Self.board())
        let fix = try #require(Self.node(tree, "theme:theme-Plan/task:t1/ticket:t1"))
        #expect(fix.children.map(\.kind) == [.subagent])
        #expect(fix.children[0].title == "Builder · Opus")
        #expect(fix.children[0].caption == "No terminal; runs inside the orchestrator")
        #expect(fix.children[0].target == .orchestrator)
        #expect(fix.children[0].id == "theme:theme-Plan/task:t1/ticket:t1/agent:a1")

        // An agent with a pane of its own is that pane, not a subagent; the orchestrator is the chat column.
        var input = try Self.board()
        input.plan.agents[0].terminal = "pane-1"
        #expect(Self.node(OneTree.build(input), "theme:theme-Plan/task:t1/ticket:t1")?.children.isEmpty == true)
        input.plan.agents[0].terminal = nil
        input.plan.agents[0].role = .orchestrator
        #expect(Self.node(OneTree.build(input), "theme:theme-Plan/task:t1/ticket:t1")?.children.isEmpty == true)
        // An agent on another ticket is not this one's, and a landed ticket lists none.
        input.plan.agents[0].role = .reviewer
        input.plan.agents[0].tickets[0] = PlanAgentTicket(task: "t2", key: "ov-2", role: .review, startedAt: 0)
        #expect(Self.node(OneTree.build(input), "theme:theme-Plan/task:t1/ticket:t1")?.children.isEmpty == true)
        #expect(Self.node(OneTree.build(input), "theme:theme-Plan/task:t2/ticket:t2")?.children.map(\.title) == ["claude", "zsh", "Reviewer · Opus"])
        input.plan.lines[1].stage = .landed
        #expect(Self.node(OneTree.build(input), "theme:theme-Plan/task:t2/ticket:t2")?.children.map(\.title) == ["claude", "zsh"])
    }

    @Test("The card's subagents stand in where the plan records none, and sit under a card with no ticket")
    func cardWorkers() throws {
        var input = try Self.board()
        let worker = TaskWorker(harness: "claude", state: .running, model: "sonnet")
        input.tasks[1].workers = [worker, TaskWorker(harness: "claude", state: .finished)]
        input.tasks[5].workers = [worker]
        let tree = OneTree.build(input)
        let copy = try #require(Self.node(tree, "theme:theme-Plan/task:t2/ticket:t2"))
        #expect(copy.children.map(\.title) == ["claude", "zsh", "Subagent · Sonnet"])
        let t5 = try #require(Self.node(tree, "group:no-theme/task:t5"))
        #expect(t5.children.map(\.kind) == [.subagent])
        // The plan's record wins: the fix round's agent isn't said twice.
        input.tasks[0].workers = [worker]
        let fix = try #require(Self.node(OneTree.build(input), "theme:theme-Plan/task:t1/ticket:t1"))
        #expect(fix.children.map(\.title) == ["Builder · Opus"])
    }

    @Test("A card's own worktree with no ticket recorded is a ticket row all the same, opening the worktree")
    func ownWorktree() throws {
        var input = try Self.board()
        input.tasks[5].worktreeID = "wt-spike"
        let tree = OneTree.build(input)
        let t5 = try #require(Self.node(tree, "group:no-theme/task:t5"))
        #expect(t5.children.map(\.title) == ["spike"])
        #expect(t5.children[0].kind == .ticket)
        #expect(t5.children[0].target == .worktree("wt-spike"))
        // And it's no longer loose.
        #expect(tree.below.map(\.title) == ["Main Checkout"])
    }

    @Test("Loose Worktrees: only those no ticket or card reaches, closed; Main Checkout holds the project terminals")
    func below() throws {
        let tree = OneTree.build(try Self.board())
        let loose = try #require(tree.below.last)
        #expect(loose.children.map(\.title) == ["spike"])
        #expect(!loose.expandedByDefault)
        let main = tree.below[0]
        #expect(main.children.map(\.title) == ["zsh", "logs"])
        #expect(main.detail == "2 shells")
        #expect(main.target == .worktree("main"))
    }

    @Test("Pages anchored to a theme sit under it; the rest sit under Plan")
    func pages() throws {
        let tree = OneTree.build(try Self.board())
        #expect(tree.places[1].children.map(\.title) == ["Landing"])
        #expect(tree.places[1].children[0].target == .page("landing"))
        // Anchored to a theme the tree doesn't show: under Plan, never lost.
        var input = try Self.board()
        input.pages.append(OneTreePage(slot: "old", title: "Old", themeID: "theme-gone"))
        #expect(OneTree.build(input).places[1].children.map(\.title) == ["Landing", "Old"])
    }

    @Test("No Theme holds the unthemed open cards, needing a decision first, then the most recently moved")
    func noTheme() throws {
        var input = try Self.board()
        input.tasks.append(Self.task("t7", .todo, at: 99))
        let tree = OneTree.build(input)
        let none = try #require(tree.tree.last)
        #expect(none.children.map(\.key) == ["ov-6", "ov-7", "ov-5"])
        #expect(none.detail == "3")
        #expect(!none.expandedByDefault)
    }

    // MARK: Expansion

    @Test("A theme with a live ticket opens by default, and in it the cards being worked; the rest show name and progress")
    func liveThemesOpen() throws {
        // Plan's t1 and t2 and Mac's t3 all have live tickets.
        var tree = OneTree.build(try Self.board())
        #expect(tree.tree.map(\.expandedByDefault) == [true, true, false])
        // t1's ticket is in a fix round and t2's is building; the fold stays shut.
        #expect(tree.tree[0].children.map(\.expandedByDefault) == [true, true, false])
        // Only t1's ticket live: Plan opens, Mac stays closed.
        var input = try Self.quietBoard()
        input.plan.lines[0].stage = .building
        tree = OneTree.build(input)
        #expect(tree.tree.map(\.expandedByDefault) == [true, false, false])
        // Nothing moving: every theme closed.
        #expect(OneTree.build(try Self.quietBoard()).tree.map(\.expandedByDefault) == [false, false, false])
        // A waiting ticket isn't moving yet: it opens neither its theme nor its card.
        input = try Self.quietBoard()
        input.plan.lines[1].stage = .waiting
        tree = OneTree.build(input)
        #expect(tree.tree.map(\.expandedByDefault) == [false, false, false])
        // One waiting on you is: its theme and its card open.
        input.plan.lines[1].stage = .needsYou
        tree = OneTree.build(input)
        #expect(tree.tree.map(\.expandedByDefault) == [true, false, false])
        #expect(tree.tree[0].children.map(\.expandedByDefault) == [false, true, false])
    }

    @Test("A theme that asks for you, or has a card that does, opens by default with no work moving")
    func askingThemesOpen() throws {
        var tree = OneTree.build(try Self.quietBoard(asks: OneTreeAsks(themes: ["theme-Mac"])))
        #expect(tree.tree.map(\.expandedByDefault) == [false, true, false])
        tree = OneTree.build(try Self.quietBoard(asks: OneTreeAsks(tasks: ["t2"])))
        #expect(tree.tree.map(\.expandedByDefault) == [true, false, false])
    }

    @Test("A paused theme opens only for the same reasons: live work or an ask")
    func pausedFollowsTheSameRule() throws {
        var input = try Self.quietBoard()
        input.plan.themes[1].state = "paused"
        #expect(OneTree.build(input).tree.map(\.expandedByDefault) == [false, false, false])
        input.plan.lines[2].stage = .building
        #expect(OneTree.build(input).tree.map(\.expandedByDefault) == [false, true, false], "Mac's t3 is live; Plan's tickets are landed")
    }

    @Test("Choices override defaults, and survive being kept as text")
    func expansionChoices() throws {
        let tree = OneTree.build(try Self.quietBoard())
        var expansion = OneTreeExpansion()
        #expect(OneTree.rows(tree.tree, expansion: expansion).map(\.node.title) == ["Plan", "Mac", "No Theme"])
        expansion.toggle(tree.tree[0])
        let rows = OneTree.rows(tree.tree, expansion: expansion)
        #expect(rows.map(\.node.title) == ["Plan", "Title t1", "Title t2", "1 done", "Mac", "No Theme"])
        #expect(rows.map(\.depth) == [0, 1, 1, 1, 0, 0])
        let kept = OneTreeExpansion(encoded: expansion.encoded)
        #expect(kept == expansion)
        #expect(OneTreeExpansion(encoded: "not json") == OneTreeExpansion())
    }

    // MARK: Needs You

    @Test("An ask's dot sits on the asking item and rolls up to each closed ancestor, and only those")
    func rollUp() throws {
        let input = try Self.board(asks: OneTreeAsks(tasks: ["t2"]))
        let tree = OneTree.build(input)
        // Everything closed: the theme carries the dot.
        var expansion = OneTreeExpansion(choices: ["theme:theme-Mac": false, "theme:theme-Plan": false])
        var rows = OneTree.rows(tree.tree, expansion: expansion)
        #expect(rows.filter(\.dot).map(\.node.title) == ["Plan"])
        // Open the theme: the dot moves to the card, and leaves the theme.
        expansion.choices["theme:theme-Plan"] = true
        rows = OneTree.rows(tree.tree, expansion: expansion)
        #expect(rows.filter(\.dot).map(\.node.title) == ["Title t2"])
        // Open the card too: the card still asks itself.
        expansion.choices["theme:theme-Plan/task:t2"] = true
        rows = OneTree.rows(tree.tree, expansion: expansion)
        #expect(rows.filter(\.dot).map(\.node.title) == ["Title t2"])
    }

    @Test("A blocked agent's pane rolls up through its ticket and card to a closed theme")
    func paneAsk() throws {
        let tree = OneTree.build(try Self.board(asks: OneTreeAsks(terminals: ["c1"])))
        // Both tickets on the worktree hold it: each of their themes rolls it up.
        let closed = OneTreeExpansion(choices: ["theme:theme-Mac": false, "theme:theme-Plan": false])
        #expect(OneTree.rows(tree.tree, expansion: closed).filter(\.dot).map(\.node.title) == ["Plan", "Mac"])
        // Open Plan: its card ov-2 carries it now, its closed ticket under it.
        let open = OneTreeExpansion(choices: ["theme:theme-Mac": false, "theme:theme-Plan": true, "theme:theme-Plan/task:t2": false])
        #expect(OneTree.rows(tree.tree, expansion: open).filter(\.dot).map(\.node.title) == ["Title t2", "Mac"])
        // Down to the pane: only the pane.
        let all = OneTreeExpansion(choices: [
            "theme:theme-Mac": false, "theme:theme-Plan": true, "theme:theme-Plan/task:t2": true,
            "theme:theme-Plan/task:t2/ticket:t2": true,
        ])
        #expect(OneTree.rows(tree.tree, expansion: all).filter(\.dot).map(\.node.title) == ["claude", "Mac"])
    }

    @Test("A theme's own ask dots the theme, open or closed")
    func themeAsk() throws {
        let tree = OneTree.build(try Self.board(asks: OneTreeAsks(themes: ["theme-Mac"])))
        for open in [true, false] {
            let rows = OneTree.rows(tree.tree, expansion: OneTreeExpansion(choices: ["theme:theme-Mac": open]))
            #expect(rows.filter(\.dot).map(\.node.title) == ["Mac"])
        }
    }

    @Test("The asks, from the Needs You list as the runner writes it: a card's, a pane's, the orchestrator's")
    func asksFromTheList() throws {
        struct Fixture: Decodable {
            struct Runner: Decodable {
                var runner: String
                var needs_you: NeedsYouList
            }
            var runners: [Runner]
        }
        let data = try Data(contentsOf: Self.root.appendingPathComponent("test/fixtures/needs-you.json"))
        let fixture = try JSONDecoder().decode(Fixture.self, from: data)
        let items = fixture.runners.flatMap(\.needs_you.items)
        let plan = try P.plan(themes: [Self.theme("A", cards: [], ordinal: 0, ask: "Which way?"), Self.theme("B", cards: [], ordinal: 1)], lines: [])
        let asks = OneTreeAsks(items: items, plan: plan)
        // Includes the task an escalated review thread is on (…0e); the pull
        // request that wants a review (ov-315) is on no task and adds none.
        #expect(asks.tasks == [
            "01000000-0000-7000-8000-000000000003", "01000000-0000-7000-8000-00000000000c",
            "01000000-0000-7000-8000-00000000000d", "01000000-0000-7000-8000-00000000000e",
            "01000000-0000-7000-8000-00000000001e",
        ])
        #expect(asks.terminals == ["01000000-0000-7000-8000-000000000016"])
        #expect(asks.orchestrator)
        #expect(asks.themes == ["theme-A"])
    }

    // MARK: Filters

    @Test("In Review: only cards in review, and only the themes holding one")
    func inReview() throws {
        let tree = OneTree.build(try Self.board(filter: .inReview))
        #expect(tree.tree.map(\.title) == ["Plan"])
        #expect(tree.tree[0].children.map(\.key) == ["ov-2"])
        #expect(tree.below.map(\.title) == ["Main Checkout", "Loose Worktrees"])
    }

    @Test("All: finished cards inline, no fold")
    func all() throws {
        let tree = OneTree.build(try Self.board(filter: .all))
        #expect(tree.tree[0].children.map(\.key) == ["ov-1", "ov-2", "ov-9"])
        #expect(tree.tree[0].children[2].quiet)
    }

    @Test("A finished theme shows under Open only while it has open cards")
    func finishedTheme() throws {
        var input = try Self.board()
        input.plan.themes[0].state = "done"
        #expect(OneTree.build(input).tree.map(\.title) == ["Mac", "Plan", "No Theme"])
        input.plan.themes[0].cards = [PlanCardRef(task: "t9", key: "ov-9")]
        input.tasks.removeAll { $0.id == "t1" || $0.id == "t2" }
        #expect(OneTree.build(input).tree.map(\.title) == ["Mac", "No Theme"])
        input.filter = .all
        #expect(OneTree.build(input).tree.map(\.title) == ["Mac", "Plan", "No Theme"])
    }

    // MARK: A board without a plan

    @Test("No plan: No Theme holds every open card, open from the start, then Main Checkout and Loose Worktrees")
    func noPlan() throws {
        var input = try Self.board()
        input.plan = .empty
        input.pages = []
        let tree = OneTree.build(input)
        #expect(tree.places.map(\.title) == ["Needs You", "Plan"])
        #expect(tree.places[1].children.isEmpty)
        #expect(tree.tree.map(\.title) == ["No Theme"])
        #expect(tree.tree[0].expandedByDefault)
        #expect(tree.tree[0].children.map(\.key) == ["ov-4", "ov-6", "ov-5", "ov-1", "ov-3", "ov-2", ""])
        // Its finished cards fold, as a theme's do (review L5).
        #expect(tree.tree[0].children.last?.title == "1 done")
        #expect(tree.tree[0].children.last?.children.map(\.id) == ["group:no-theme/task:t9"])
        // No tickets: every worktree is loose.
        #expect(tree.below.last?.children.map(\.title) == ["primary", "copy", "spike"])
        let rows = OneTree.rows(tree.roots, expansion: OneTreeExpansion())
        #expect(rows.contains { $0.node.key == "ov-4" })
    }

    @Test("An empty board: the places and nothing else; the orchestrator is the chat column, never a row (R-14)")
    func emptyBoard() {
        let tree = OneTree.build(OneTreeInput(tasks: []))
        #expect(tree.places.map(\.title) == ["Needs You", "Plan"])
        #expect(tree.tree.isEmpty)
        #expect(tree.below.isEmpty)
    }

    // MARK: Paths

    @Test("The path: Theme › Task › Ticket › Terminal, the hinted copy of a worktree shared by two tickets")
    func path() throws {
        let tree = OneTree.build(try Self.board())
        let terminal = OneTreeTarget.terminal(worktree: "wt-copy", terminal: "c2")
        #expect(tree.crumbs(to: terminal).map(\.title) == ["Plan", "ov-2 Title t2", "ov-2", "zsh"])
        let mac = "theme:theme-Mac/task:t3/ticket:t3/terminal:c2"
        #expect(tree.crumbs(to: terminal, hint: mac).map(\.title) == ["Mac", "ov-3 Title t3", "ov-3", "zsh"])
        #expect(tree.crumbs(to: terminal, hint: mac).map(\.target) == [
            .theme("theme-Mac"), .task("t3"), .ticket("t3"), terminal,
        ])
        #expect(tree.ancestors(of: terminal, hint: mac) == [
            "theme:theme-Mac", "theme:theme-Mac/task:t3", "theme:theme-Mac/task:t3/ticket:t3",
        ])
    }

    @Test("A ticket's worktree opened whole is found as its ticket; a worktree with no ticket, as itself")
    func worktreeAsTicket() throws {
        let tree = OneTree.build(try Self.board())
        #expect(tree.crumbs(to: .worktree("wt-copy")).map(\.title) == ["Plan", "ov-2 Title t2", "ov-2"])
        #expect(tree.parent(of: .worktree("wt-copy")) == .task("t2"))
        #expect(tree.crumbs(to: .worktree("wt-spike")).map(\.title) == ["Loose Worktrees", "spike"])
        #expect(tree.path(to: .worktree("wt-nowhere")) == nil)
        let copy = try #require(Self.node(tree, "theme:theme-Plan/task:t2/ticket:t2"))
        #expect(copy.stands(for: .worktree("wt-copy")))
        #expect(copy.stands(for: .ticket("t2")))
        #expect(!copy.stands(for: .worktree("wt-primary")))
        #expect(!copy.stands(for: nil))
    }

    @Test("⌘↑: the parent that goes somewhere; a theme's is the plan; the plan has none")
    func parent() throws {
        let tree = OneTree.build(try Self.board())
        #expect(tree.parent(of: .ticket("t2")) == .task("t2"))
        #expect(tree.parent(of: .ticket("t3"), hint: "theme:theme-Mac/task:t3/ticket:t3") == .task("t3"))
        #expect(tree.parent(of: .task("t9")) == .theme("theme-Plan"))
        #expect(tree.parent(of: .theme("theme-Mac")) == .plan)
        #expect(tree.parent(of: .task("t5")) == .plan)
        #expect(tree.parent(of: .page("landing")) == .plan)
        #expect(tree.parent(of: .plan) == nil)
        #expect(tree.parent(of: .task("nowhere")) == nil)
    }

    // MARK: Words

    @Test("Statuses as plan --json says them, and as the wire does")
    func statusWords() {
        #expect(OneTreeTask.status(word: "In Review") == .inReview)
        #expect(OneTreeTask.status(word: "Done") == .done)
        #expect(OneTreeTask.status(word: "To Do") == .todo)
        #expect(OneTreeTask.status(word: "Needs Decision") == .needsDecision)
        #expect(OneTreeTask.status(word: "Cancelled") == .cancelled)
        #expect(OneTreeTask.status(word: "Canceled") == .cancelled)
        #expect(OneTreeTask.status(word: "in_progress") == .inProgress)
        #expect(OneTreeTask.status(word: "Someday") == nil)
    }

    @Test("Also, shells and progress, in words")
    func words() {
        #expect(OneTreeWords.also([]) == "")
        #expect(OneTreeWords.also(["ov-303"]) == "also ov-303")
        #expect(OneTreeWords.also(["ov-303", "ov-304", "ov-305"]) == "also ov-303 +2")
        #expect(OneTreeWords.shells(0) == "No shells")
        #expect(OneTreeWords.shells(1) == "1 shell")
        #expect(OneTreeWords.shells(3) == "3 shells")
    }

    // MARK: A real board

    @Test("A real board's plan: every ticket under its card, a theme's ask on the theme")
    func seededBoard() throws {
        let plan = try Self.seeded()
        let asks = OneTreeAsks(items: [], plan: plan)
        let tree = OneTree.build(OneTreeInput(tasks: [], plan: plan, asks: asks))
        // The cards come from the plan's own read: no board needed.
        let visual = try #require(tree.tree.first)
        #expect(visual.title == "Visual language")
        #expect(visual.asks)
        // Seven tickets are building on mac-vis: a row under each open card, none under a finished one.
        let onMacVis = plan.lines.filter { $0.branch == "mac-vis" }
        #expect(onMacVis.count == 7)
        let open = onMacVis.filter { line in
            plan.cards.first { $0.task == line.task }.flatMap(OneTreeTask.init(card:)).map { !$0.status.isFinished } ?? false
        }
        let rows = tree.allNodes.filter { node in
            guard node.kind == .ticket, case .ticket(let task)? = node.target else { return false }
            return onMacVis.contains { $0.task == task }
        }
        #expect(!open.isEmpty && rows.count == open.count)
        #expect(Set(rows.map(\.target)) == Set(open.map { OneTreeTarget.ticket($0.task) }))
        #expect(rows.allSatisfy { $0.detail == "Building" && $0.ticketStage == .building && $0.also.isEmpty })
        // Waiting tickets have no worktree yet: no terminals under them.
        let waiting = tree.allNodes.filter { $0.kind == .ticket && $0.ticketStage == .waiting }
        #expect(!waiting.isEmpty)
        #expect(waiting.allSatisfy { $0.children.isEmpty })
        // Each theme's progress, as its counts say.
        #expect(tree.tree.prefix(5).map(\.detail) == plan.shownThemes.map { OneTreeWords.progress($0.counts) })
    }

    // MARK: Narrowing

    @Test("⌘F narrows to the matches and the path to each, all open; no text cuts nothing")
    func narrowing() throws {
        let tree = OneTree.build(try Self.board())
        #expect(OneTree.narrowed(tree.tree, to: "  ") == tree.tree)
        let kept = OneTree.narrowed(tree.tree, to: "ZSH")
        let rows = OneTree.rows(kept, expansion: OneTree.allOpen(kept))
        #expect(rows.map(\.node.title) == ["Plan", "Title t2", "ov-2", "zsh", "Mac", "Title t3", "ov-3", "zsh"])
        // A key matches too, and a match keeps its own children.
        let byKey = OneTree.narrowed(tree.tree, to: "ov-1")
        #expect(byKey.map(\.title) == ["Plan"])
        #expect(byKey[0].children.map(\.key) == ["ov-1"])
        #expect(byKey[0].children[0].children.map(\.title) == ["ov-1"])
        #expect(OneTree.narrowed(tree.tree, to: "nothing like it").isEmpty)
    }
}
