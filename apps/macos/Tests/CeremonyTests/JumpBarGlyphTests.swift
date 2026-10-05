import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The jump bar draws the sidebar's icons (ov-328): each segment and each
/// menu row has the SF Symbol its row has in the navigator. The sidebar's
/// rows read `OneTreeNode.glyph`, built by `OneTree`; the bar's segments read
/// `ContentView.crumbs(top:through:resolve:)`, and its menu rows the
/// `JumpMenus` builders. Each test reads both sides through those call
/// sites, on the real plan the CLI wrote (`plan-seeded.json`), so a symbol
/// changed at one site and not the other turns it red.
@MainActor
struct JumpBarGlyphTests {
    private typealias Selection = ContentView.Selection

    private static var root: URL {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return root
    }

    /// A tree over the seeded plan: every theme, the lanes that have a
    /// worktree with an agent and a shell open, a task of every status, a loose
    /// worktree, the main checkout and a page. The filter is All, as the jump
    /// bar's tree is (`pathTree`).
    private static func tree() throws -> (tree: OneTree, tasks: [OneTreeTask]) {
        struct Seeded: Decodable { var plan: PlanModel }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/plan-seeded.json"))
        let decoder = JSONDecoder()
        decoder.keyDecodingStrategy = .convertFromSnakeCase
        let plan = try decoder.decode(Seeded.self, from: data).plan
        let statuses = TaskStatus.allCases
        var seen = Set<String>()
        let cards = plan.themes.flatMap(\.cards).map(\.task).filter { seen.insert($0).inserted }
        let tasks = cards.enumerated().map { index, id in
            OneTreeTask(id: id, key: "ov-\(index)", title: "Card \(index)", status: statuses[index % statuses.count])
        }
        func worktree(_ name: String, path: String) -> OneTreeWorktree {
            OneTreeWorktree(
                id: "wt-\(name)", name: name, path: path, branch: name,
                terminals: [
                    OneTreeTerminal(id: "\(name)-agent", title: "claude", isAgent: true),
                    OneTreeTerminal(id: "\(name)-shell", title: "zsh", isAgent: false),
                ])
        }
        let lanes = plan.lanes.filter { !$0.worktreePath.isEmpty }.map { worktree($0.name, path: $0.worktreePath) }
        let tree = OneTree.build(
            OneTreeInput(
                tasks: tasks, plan: plan, worktrees: lanes + [worktree("spike", path: "/repo/spike")],
                mainCheckout: OneTreeWorktree(
                    id: "main", name: "repo", isMainCheckout: true,
                    terminals: [OneTreeTerminal(id: "main-shell", title: "zsh", isAgent: false)]),
                pages: [OneTreePage(slot: "train", title: "Train")], filter: .all, needsYouCount: 1))
        return (tree, tasks)
    }

    /// The crumbs the bar shows for `node`'s target, as the window builds them.
    private static func crumbs(_ tree: OneTree, to node: OneTreeNode) -> [WorkspaceNavigation.Crumb] {
        let path = tree.crumbs(to: node.target!, hint: node.id)
        let top = WorkspaceNavigation.Crumb(title: "Billing", target: nil)
        return ContentView.crumbs(top: top, through: path) { _ in nil }
    }

    @Test("Every segment the bar can show has its sidebar row's symbol, on a tree with every kind of row")
    func everySegmentMatchesItsRow() throws {
        let (tree, _) = try Self.tree()
        let reached = tree.allNodes.filter { $0.target != nil && $0.kind != .place && $0.kind != .doneFold }
        // The tree is rich enough to mean it: each kind the bar can name.
        let kinds = Set(reached.map(\.kind))
        for kind: OneTreeNode.Kind in [.theme, .task, .lane, .terminal, .page, .group, .worktree] {
            #expect(kinds.contains(kind), "\(kind) missing from the fixture")
        }
        for node in reached {
            let crumbs = Self.crumbs(tree, to: node)
            // The path's own glyphs are each node's, so the last segment is
            // this row; the workspace's, first, has no row to match.
            #expect(crumbs.first?.glyph == nil)
            #expect(!node.glyph.isEmpty, "\(node.id)")
            #expect(crumbs.last?.glyph == node.glyph, "\(node.kind) \(node.id)")
            // Every segment on the way is a row of the tree, with its symbol.
            for (crumb, row) in zip(crumbs.dropFirst(), tree.path(to: node.target!, hint: node.id)!.filter { $0.kind != .place && $0.kind != .doneFold }) {
                #expect(crumb.glyph == row.glyph, "\(node.id) via \(row.id)")
            }
        }
    }

    @Test("A task's menu row has the symbol of its status, as the sidebar's row for a card in that status")
    func taskRowsMatch() throws {
        let (tree, cards) = try Self.tree()
        let tasks = tree.allNodes.filter { $0.kind == .task && $0.id.hasPrefix("theme:") }
        var statuses = Set<TaskStatus>()
        for node in tasks {
            guard case .task(let id)? = node.target, let card = cards.first(where: { $0.id == id }) else { continue }
            statuses.insert(card.status)
            let row = TaskRow(id: id, key: card.key, title: card.title, status: card.status, statusSince: Date(timeIntervalSince1970: 0))
            let menu = JumpMenus.workspaceLevel(
                host: "", workspace: "w", place: .workspace(host: "", workspace: "w", focus: nil), hasOrchestrator: false,
                orchestrator: nil,
                board: TaskBoardModel(columns: [TaskBoardColumn(status: card.status, rows: [row])]),
                taskStatus: { _ in nil }, taskWorktree: { _ in nil }, loose: [], worktreeStatus: { _ in nil },
                opening: { _ in .workspace(host: "", workspace: "w", focus: nil) }, tab: nil)
            #expect(menu.items.first { $0.id == "task|\(id)" }?.glyph == node.glyph, "\(card.status)")
        }
        #expect(statuses == Set(TaskStatus.allCases), "a card of every status")
    }

    @Test("The orchestrator's and a loose worktree's menu rows have their sidebar symbols")
    func orchestratorAndWorktreeRowsMatch() throws {
        let (tree, _) = try Self.tree()
        let orchestrator = try #require(tree.places.first { $0.target == .orchestrator })
        let loose = try #require(tree.below.last?.children.first { $0.kind == .worktree })
        let wt = Worktree(
            id: "wt", short: "wt", task: "spike", branch: "spike", repository: "r", host: "", path: "/tmp/wt",
            state: "active", terminals: [], repositoryID: "r", workspace: "w")
        let menu = JumpMenus.workspaceLevel(
            host: "", workspace: "w", place: .workspace(host: "", workspace: "w", focus: nil), hasOrchestrator: true,
            orchestrator: nil, board: TaskBoardModel(columns: []), taskStatus: { _ in nil }, taskWorktree: { _ in nil },
            loose: [wt], worktreeStatus: { _ in nil }, opening: { _ in .workspace(host: "", workspace: "w", focus: nil) },
            tab: nil)
        #expect(menu.items.first { $0.id == "orchestrator" }?.glyph == orchestrator.glyph)
        #expect(menu.items.first { $0.id == "wt|wt" }?.glyph == loose.glyph)
        let sibling = WorkspaceWorktrees.MenuItem(
            title: "spike", subtitle: nil, target: .workspace(host: "", workspace: "w", focus: nil), current: false)
        let siblings = JumpMenus.worktree(siblings: (tasks: [], loose: [sibling]), children: [], actions: [], named: nil)
        #expect(siblings.items.first?.glyph == loose.glyph)
    }

    @Test("A terminal's menu row has its sidebar symbol: an agent's sparkles, a shell's prompt")
    func terminalRowsMatch() throws {
        let (tree, _) = try Self.tree()
        let sidebar = tree.allNodes.filter { $0.kind == .terminal }
        #expect(Set(sidebar.map(\.glyph)).count == 2, "an agent and a shell in the fixture")
        var agent = Terminal(id: "a", short: "a", title: "claude", preset: "claude", state: "running", epoch: 0)
        agent.paneMode = "agent"
        let shell = Terminal(id: "s", short: "s", title: "zsh", preset: "zsh", state: "running", epoch: 0)
        let wt = Worktree(
            id: "wt", short: "wt", task: "wt", branch: "wt", repository: "r", host: "", path: "/tmp/wt", state: "active",
            terminals: [agent, shell], repositoryID: "r", workspace: "w")
        let fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [wt], branchPrefix: nil)
        let items = JumpMenus.terminals(of: wt, selection: nil, fleet: fleet).flatMap(\.items)
        let treeAgent = ContentView.treeTerminal(agent)
        let treeShell = ContentView.treeTerminal(shell)
        let glyph = { (t: OneTreeTerminal) in
            OneTree.build(OneTreeInput(tasks: [], worktrees: [OneTreeWorktree(id: "x", name: "x", terminals: [t])], filter: .all))
                .allNodes.first { $0.kind == .terminal }!.glyph
        }
        #expect(items.first { $0.id == "term|a" }?.glyph == glyph(treeAgent))
        #expect(items.first { $0.id == "term|s" }?.glyph == glyph(treeShell))
        #expect(items.first { $0.id == "term|a" }?.glyph != items.first { $0.id == "term|s" }?.glyph)
    }

    @Test("Menu rows with no row in the sidebar draw no icon: workspaces, tabs, the way to History")
    func rowsWithNoSidebarRow() {
        let groups = [
            WorkspaceNumbers.Group(
                host: "", repository: "shop",
                places: [
                    .init(
                        host: "", workspace: WorkspaceSummary(id: "w", name: "Billing", taskPrefix: "b", isMain: false, ordinal: 1, repository: "r"),
                        name: "Billing", number: 1)
                ])
        ]
        let menu = JumpMenus.workspaces(groups, current: nil, showsHosts: false, waiting: { _ in 0 })
        #expect(menu.items.allSatisfy { $0.glyph == nil })
    }
}
