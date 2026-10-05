import Foundation
import Testing

@testable import AgentKit

/// The one tree's first fix round (ov-321, the train-1005b review): a build
/// linear in the board, the keyboard reaching every row, the default open
/// theme taken once, one Needs You count, and what the tree had dropped.
struct OneTreeRoundOneTests {
    typealias T = OneTreeTests
    typealias P = PlanModelTests

    // MARK: H2, a build linear in the board

    /// A board as big as a long-lived one gets: `cards` cards in 20
    /// themes, lanes of two cards each (one in three live), a worktree per
    /// live lane with two panes. Through `PlanModel.decode`, the app's parser.
    static func big(cards: Int, lanes: Int) throws -> OneTreeInput {
        let ids = (0..<cards).map { "t\($0)" }
        let themes = (0..<20).map { n in
            T.theme("Theme \(n)", cards: stride(from: n, to: cards, by: 20).map { ids[$0] }, ordinal: n)
        }
        let states = ["building", "landed", "queued"]
        let laneRows = (0..<lanes).map { n in
            T.lane("lane\(n)", states[n % 3], cards: [ids[(2 * n) % cards], ids[(2 * n + 1) % cards]],
                path: ".claude/worktrees/lane\(n)")
        }
        let plan = try P.plan(themes: themes, lanes: laneRows)
        let statuses: [TaskStatus] = [.inProgress, .done, .todo, .inReview, .backlog]
        return OneTreeInput(
            tasks: ids.enumerated().map { n, id in T.task(id, statuses[n % statuses.count], at: Int64(n)) },
            plan: plan,
            worktrees: (0..<lanes).filter { $0 % 3 == 0 }.map { n in
                T.worktree(
                    "lane\(n)",
                    terminals: [
                        OneTreeTerminal(id: "a\(n)", title: "claude", isAgent: true),
                        OneTreeTerminal(id: "s\(n)", title: "zsh", isAgent: false),
                    ])
            })
    }

    @Test("1,000 cards and 300 lanes build in well under a frame budget, even in a debug build on CI")
    func bigBoardBuildsFast() throws {
        let input = try Self.big(cards: 1000, lanes: 300)
        let clock = ContinuousClock()
        var tree = OneTree.build(input)
        let took = clock.measure { tree = OneTree.build(input) }
        // Measured on this Mac, debug: about 7 ms (it was 200 ms before the
        // joins were indexed). A quarter second leaves a slow CI runner 20x.
        #expect(took < .milliseconds(250), "took \(took)")
        print("one tree, 1,000 cards: \(took)")
        #expect(tree.tree.count == 20)
        // Every live lane's two cards list it: the indexes join what the
        // scans did.
        #expect(tree.allNodes.filter { $0.target == .lane("lane-lane0") }.count == 2)
    }

    @Test("Doubling the board about doubles the build, not quadruples it")
    func buildIsLinear() throws {
        let small = try Self.big(cards: 1000, lanes: 300), large = try Self.big(cards: 4000, lanes: 1200)
        let clock = ContinuousClock()
        func best(_ input: OneTreeInput) -> Duration {
            (0..<3).map { _ in clock.measure { _ = OneTree.build(input) } }.min()!
        }
        _ = OneTree.build(small)
        let ratio = best(large) / best(small)
        // Linear is 4; quadratic was 16. Eight leaves room for noise.
        #expect(ratio < 8, "4x the board took \(ratio)x as long")
    }

    // MARK: H1, every row from the keyboard

    @Test("↓ walks every row, groups with nowhere to go included; → opens, ← closes and climbs")
    func keyboardReachesEveryRow() throws {
        let tree = OneTree.build(try T.board())
        var expansion = OneTreeExpansion(choices: ["theme:theme-Mac": false])
        var rows = OneTree.rows(tree.roots, expansion: expansion)
        // From nothing, ↓ is the first row; walk to No Theme.
        var cursor: String?
        var seen: [String] = []
        while case .cursor(let next) = OneTreeKeys.step(rows, from: cursor, by: 1) {
            cursor = next
            seen.append(next)
        }
        #expect(seen == rows.map(\.id))
        #expect(seen.contains("group:no-theme") && seen.contains("group:loose"))
        // → on No Theme opens it; → again moves to its first card.
        #expect(OneTreeKeys.right(rows, cursor: "group:no-theme") == .toggle("group:no-theme"))
        expansion.toggle(try #require(tree.tree.last))
        rows = OneTree.rows(tree.roots, expansion: expansion)
        #expect(OneTreeKeys.right(rows, cursor: "group:no-theme") == .cursor("group:no-theme/task:t6"))
        // ← on that card climbs to No Theme; ← on No Theme closes it.
        #expect(OneTreeKeys.left(rows, cursor: "group:no-theme/task:t6") == .cursor("group:no-theme"))
        #expect(OneTreeKeys.left(rows, cursor: "group:no-theme") == .toggle("group:no-theme"))
        // Return opens a group, chooses a row that goes somewhere.
        #expect(OneTreeKeys.enter(rows, cursor: "group:loose") == .toggle("group:loose"))
        #expect(OneTreeKeys.enter(rows, cursor: "group:no-theme/task:t6") == .choose("group:no-theme/task:t6"))
        // ↑ at the top and ↓ at the bottom stay put.
        #expect(OneTreeKeys.step(rows, from: rows.first?.id, by: -1) == .none)
        #expect(OneTreeKeys.step(rows, from: rows.last?.id, by: 1) == .none)
    }

    @Test("Arriving on a subagent doesn't take the keyboard into the chat; the first place is Needs You, not an Orchestrator row")
    func arrivalNeverLeaves() throws {
        let tree = OneTree.build(try T.board())
        #expect(try #require(tree.places.first).target == .needsYou)
        let subagent = try #require(tree.allNodes.first { $0.kind == .subagent })
        #expect(!OneTreeKeys.choosesOnArrival(subagent))
        #expect(!OneTreeKeys.choosesOnArrival(try #require(tree.tree.last)))
        #expect(OneTreeKeys.choosesOnArrival(try #require(T.node(tree, "theme:theme-Plan/task:t1"))))
    }

    // MARK: M1, the default taken once

    @Test("The open theme is taken once and stays as activity moves")
    func defaultIsSticky() throws {
        var input = try T.board()
        var expansion = OneTreeExpansion()
        expansion.seed(from: OneTree.build(input).roots)
        #expect(expansion.isSeeded)
        // Mac was newest; now Plan's card moves, and Plan would be the default.
        input.tasks[0].activityMs = P.now + 1
        let moved = OneTree.build(input)
        #expect(moved.tree[0].expandedByDefault)
        let open = OneTree.rows(moved.tree, expansion: expansion).filter(\.expanded).map(\.id)
        #expect(open == ["theme:theme-Mac", "theme:theme-Mac/task:t3"])
        // Seeding again does nothing; a choice made stays.
        expansion.toggle(moved.tree[1])
        let kept = expansion
        expansion.seed(from: moved.roots)
        #expect(expansion == kept)
        #expect(OneTreeExpansion(encoded: expansion.encoded).isSeeded)
    }

    // MARK: H3, one count

    @Test("One count: the runner's list once read; before that, or with none served, the column beside it; theme asks always")
    func oneCount() {
        let count = WorkspaceNeedsYou.count
        #expect(count(3, 5, true, true, 0) == 3)
        #expect(count(0, 5, false, true, 0) == 5)
        #expect(count(1, 3, true, false, 0) == 4)
        #expect(count(1, 3, false, false, 1) == 5)
        #expect(count(2, 0, true, true, 2) == 4)
    }

    // MARK: What was dropped

    @Test("Hidden worktrees sit in a closed Hidden group under Loose, not nowhere")
    func hiddenWorktrees() throws {
        var input = try T.board()
        input.worktrees.append(OneTreeWorktree(id: "wt-old", name: "old", isHidden: true))
        let loose = try #require(OneTree.build(input).below.last)
        #expect(loose.children.map(\.title) == ["spike", "Hidden"])
        #expect(loose.detail == "1")
        let hidden = try #require(loose.children.last)
        #expect(!hidden.expandedByDefault)
        #expect(hidden.children.map(\.worktreeID) == ["wt-old"])
    }

    @Test("A card in a status this build doesn't know is listed, not dropped")
    func unreadableCards() throws {
        var input = try T.board()
        input.unreadable = [OneTreeUnreadable(id: "u1", key: "ov-77", title: "From the future", status: "parked_forever")]
        let group = try #require(OneTree.build(input).tree.last)
        #expect(group.title == "Not On This Version")
        #expect(group.children.map(\.key) == ["ov-77"])
        #expect(group.children[0].detail == "parked_forever")
        #expect(group.children[0].target == nil)
    }

    @Test("A card's own subagents sit on its newest live lane only, the same under each copy of that lane")
    func workersOnce() throws {
        var input = try T.board()
        let worker = TaskWorker(harness: "claude", state: .running, model: "sonnet")
        input.plan.lanes[2].agents = []
        input.tasks[0].workers = [worker]
        let tree = OneTree.build(input)
        let t1 = try #require(T.node(tree, "theme:theme-Plan/task:t1"))
        #expect(t1.children.map { $0.children.filter { $0.kind == .subagent }.count } == [0, 1])
        // copy works t2 and t3; t2's worker shows under both copies.
        input.tasks[1].workers = [worker]
        let copies = OneTree.build(input).allNodes.filter { $0.target == .lane("lane-copy") }
        #expect(copies.map { $0.children.filter { $0.kind == .subagent }.count } == [1, 1])
    }

    @Test("A finished card's id is the same under the fold and in the All tree")
    func foldIDs() throws {
        let open = OneTree.build(try T.board())
        let all = OneTree.build(try T.board(filter: .all))
        let folded = try #require(open.allNodes.first { $0.target == .task("t9") })
        #expect(all.allNodes.contains { $0.id == folded.id })
    }

    @Test("Lanes, worktrees and terminals carry the worktree their menu acts on")
    func worktreeIDs() throws {
        let tree = OneTree.build(try T.board())
        let copy = try #require(T.node(tree, "theme:theme-Plan/task:t2/lane:lane-copy"))
        #expect(copy.worktreeID == "wt-copy")
        #expect(copy.children.map(\.worktreeID) == ["wt-copy", "wt-copy"])
        #expect(tree.below.last?.children.first?.worktreeID == "wt-spike")
    }
}
