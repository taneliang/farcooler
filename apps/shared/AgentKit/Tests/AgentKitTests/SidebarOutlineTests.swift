import Foundation
import Testing

@testable import AgentKit

/// The sidebar as two lists (ov-503, design 3): Themes › Cards and Lanes ›
/// Agents, built over the miniature board `OneTreeTests` seeds. Every plan
/// goes through `PlanModel.decode`.
struct SidebarOutlineTests {
    typealias T = OneTreeTests

    static func outline(filter: OneTreeFilter = .open) throws -> SidebarOutline {
        SidebarOutline.build(try T.board(filter: filter))
    }

    static func node(_ outline: SidebarOutline, _ id: String) -> OneTreeNode? { outline.allNodes.first { $0.id == id } }

    @Test("Top to bottom: the places, the themes and their cards, the lanes by group, then the checkout and loose worktrees")
    func theShape() throws {
        let outline = try Self.outline()
        #expect(outline.places.map(\.title) == ["Needs You", "Plan"])
        #expect(outline.themes.map(\.title) == ["Plan", "Mac", "No Theme"])
        // Landing is past review; the fix round is its own lane.
        #expect(outline.lanes.map(\.title).first.map { !$0.isEmpty } == true)
        #expect(outline.below.map(\.title) == ["Main Checkout", "Loose Worktrees"])
    }

    @Test("A card's row carries the lane working it; the lane's row says it combines two cards")
    func theJoinIsSaid() throws {
        let outline = try Self.outline()
        let card = try #require(Self.node(outline, "theme:theme-Plan/task:t2"))
        #expect(card.detail == "copy")
        let lane = try #require(Self.node(outline, "lane:\(try lane("copy"))"))
        #expect(lane.also == "ov-2 ov-3")
        let single = try #require(outline.allNodes.first { $0.slug == "primary" })
        #expect(single.also == "", "one card needs no words")
    }

    func lane(_ name: String) throws -> String {
        try #require(T.board().plan.lanes.first { $0.name == name }).id
    }

    @Test("Choosing a card lights its lane, and choosing a lane lights each of its cards, never the chosen row")
    func crossHighlight() throws {
        let outline = try Self.outline()
        let copy = try lane("copy")
        let fromCard = outline.related(to: .task("t2"))
        #expect(fromCard == ["lane:\(copy)"])
        let fromLane = outline.related(to: .lane(copy))
        #expect(fromLane == ["theme:theme-Plan/task:t2", "theme:theme-Mac/task:t3"])
        // The other card of the same lane lights the same lane.
        #expect(outline.related(to: .task("t3")) == fromCard)
        #expect(outline.related(to: nil).isEmpty)
        #expect(outline.related(to: .theme("theme-Plan")).isEmpty)
    }

    @Test("A lane's terminal is its lane: choosing it lights the lane and its cards")
    func terminalLightsItsLane() throws {
        let outline = try Self.outline()
        let copy = try lane("copy")
        let lit = outline.related(to: .terminal(worktree: "wt-copy", terminal: "c2"))
        #expect(lit == ["lane:\(copy)", "theme:theme-Plan/task:t2", "theme:theme-Mac/task:t3"])
    }

    @Test("A lane's worktree panes hang under it; the shell stays a terminal row")
    func terminalsUnderLanes() throws {
        let outline = try Self.outline()
        let copy = try #require(Self.node(outline, "lane:\(try lane("copy"))"))
        #expect(copy.worktreeID == "wt-copy")
        let panes = copy.children.compactMap { node -> String? in
            if case .terminal(_, let pane)? = node.target { return pane }
            return nil
        }
        #expect(panes.sorted() == ["c1", "c2"])
    }

    @Test("A worktree holding an open card with no lane is listed under Without a Lane, so its panes stay reachable")
    func worktreeWithoutALane() throws {
        var input = try T.board()
        input.tasks.append(T.task("t7", .inProgress, worktree: "wt-solo"))
        input.worktrees.append(T.worktree("solo", terminals: [OneTreeTerminal(id: "s1", title: "zsh", isAgent: false)], tasks: ["t7"]))
        let outline = SidebarOutline.build(input)
        let group = try #require(outline.lanes.last)
        #expect(group.title == "Without a Lane")
        #expect(group.children.map(\.target) == [.worktree("wt-solo")])
        #expect(!(outline.below.last?.children.contains { $0.target == .worktree("wt-solo") } ?? false), "not also loose")
    }

    @Test("The filter narrows the Themes list alone; the lanes stay")
    func filterIsForThemes() throws {
        let open = try Self.outline(filter: .open)
        let review = try Self.outline(filter: .inReview)
        #expect(open.lanes == review.lanes)
        #expect(review.themes.map(\.title) == ["Plan"], "only the card in review")
    }

    @Test("Revealing a card or a lane opens the group or theme over it")
    func ancestorsOpenTheWay() throws {
        let outline = try Self.outline()
        let copy = try lane("copy")
        #expect(outline.ancestors(of: .task("t3")) == ["theme:theme-Mac"])
        let path = outline.ancestors(of: .lane(copy))
        #expect(path.count == 1 && path[0].hasPrefix("lanes:"), "\(path)")
    }
}
