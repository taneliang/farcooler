import Foundation
import Testing

@testable import AgentKit

/// The sidebar as two lists (ov-503, design 3): Themes › Cards and Tickets ›
/// Agents, built over the miniature board `OneTreeTests` seeds. Every plan
/// goes through `PlanModel.decode`.
struct SidebarOutlineTests {
    typealias T = OneTreeTests

    static func outline(filter: OneTreeFilter = .open) throws -> SidebarOutline {
        SidebarOutline.build(try T.board(filter: filter))
    }

    static func node(_ outline: SidebarOutline, _ id: String) -> OneTreeNode? { outline.allNodes.first { $0.id == id } }

    @Test("Top to bottom: the places, the themes and their cards, the tickets by group, then the checkout and loose worktrees")
    func theShape() throws {
        let outline = try Self.outline()
        #expect(outline.places.map(\.title) == ["Needs You", "Plan"])
        #expect(outline.themes.map(\.title) == ["Plan", "Mac", "No Theme"])
        // t1 is in a fix round, so it is In Review; t2 and t3 are building.
        #expect(outline.tickets.map(\.title) == ["In Review", "Building"])
        #expect(outline.tickets[0].children.map(\.key) == ["ov-1"])
        #expect(outline.tickets[1].children.map(\.key) == ["ov-2", "ov-3"])
        #expect(outline.below.map(\.title) == ["Main Checkout", "Loose Worktrees"])
    }

    @Test("A card's row carries the stage of its ticket; the ticket's row is one card, with its key, its stage and no extra words")
    func theJoinIsSaid() throws {
        let outline = try Self.outline()
        let card = try #require(Self.node(outline, "theme:theme-Plan/task:t2"))
        #expect(card.detail == "Building")
        let ticket = try #require(Self.node(outline, "ticket:t2"))
        #expect(ticket.key == "ov-2" && ticket.slug == "ov-2" && ticket.title == "Title t2" && ticket.detail == "Building")
        #expect(ticket.caption == "", "a ticket is one card, so it needs no words for the rest")
        let fixing = try #require(Self.node(outline, "ticket:t1"))
        #expect(fixing.detail == "Fixing", "a fix round reads Fixing on its ticket")
        #expect(try #require(Self.node(outline, "theme:theme-Plan/task:t1")).detail == "Fixing", "and on its card")
    }

    @Test("Choosing a card lights its ticket, and choosing a ticket lights its card, never the chosen row")
    func crossHighlight() throws {
        let outline = try Self.outline()
        #expect(outline.related(to: .task("t2")) == ["ticket:t2"])
        #expect(outline.related(to: .ticket("t2")) == ["theme:theme-Plan/task:t2"])
        #expect(outline.related(to: .task("t3")) == ["ticket:t3"], "t3 shares t2's worktree and has a ticket of its own")
        #expect(outline.related(to: nil).isEmpty)
        #expect(outline.related(to: .theme("theme-Plan")).isEmpty)
        #expect(outline.related(to: .task("t4")).isEmpty, "a card with no ticket lights nothing")
    }

    @Test("A ticket's terminal is its ticket: choosing it lights every ticket on the worktree and their cards")
    func terminalLightsItsTicket() throws {
        let outline = try Self.outline()
        let lit = outline.related(to: .terminal(worktree: "wt-copy", terminal: "c2"))
        #expect(lit == ["ticket:t2", "theme:theme-Plan/task:t2", "ticket:t3", "theme:theme-Mac/task:t3"])
    }

    @Test("A ticket's worktree panes hang under it; the shell stays a terminal row")
    func terminalsUnderTickets() throws {
        let outline = try Self.outline()
        let copy = try #require(Self.node(outline, "ticket:t2"))
        #expect(copy.worktreeID == "wt-copy")
        let panes = copy.children.compactMap { node -> String? in
            if case .terminal(_, let pane)? = node.target { return pane }
            return nil
        }
        #expect(panes.sorted() == ["c1", "c2"])
    }

    @Test("A worktree holding an open card with no ticket is listed under Off the Line, so its panes stay reachable")
    func worktreeWithoutATicket() throws {
        var input = try T.board()
        input.tasks.append(T.task("t7", .inProgress, worktree: "wt-solo"))
        input.worktrees.append(T.worktree("solo", terminals: [OneTreeTerminal(id: "s1", title: "zsh", isAgent: false)], tasks: ["t7"]))
        let outline = SidebarOutline.build(input)
        let group = try #require(outline.tickets.last)
        #expect(group.title == "Off the Line")
        #expect(group.children.map(\.target) == [.worktree("wt-solo")])
        #expect(!(outline.below.last?.children.contains { $0.target == .worktree("wt-solo") } ?? false), "not also loose")
    }

    @Test("The filter narrows the Themes list alone; the tickets stay")
    func filterIsForThemes() throws {
        let open = try Self.outline(filter: .open)
        let review = try Self.outline(filter: .inReview)
        #expect(open.tickets == review.tickets)
        #expect(review.themes.map(\.title) == ["Plan"], "only the card in review")
    }

    @Test("Revealing a card or a ticket opens the group or theme over it")
    func ancestorsOpenTheWay() throws {
        let outline = try Self.outline()
        #expect(outline.ancestors(of: .task("t3")) == ["theme:theme-Mac"])
        let path = outline.ancestors(of: .ticket("t2"))
        #expect(path == ["tickets:building"], "\(path)")
    }
}
