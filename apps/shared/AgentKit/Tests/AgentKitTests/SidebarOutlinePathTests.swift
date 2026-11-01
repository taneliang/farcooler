import Foundation
import Testing

@testable import AgentKit

/// The jump bar's path and ⌘↑ walk the sidebar's outline (ov-512), not
/// `OneTree`'s Theme › Task › Ticket › Terminal: a card's crumbs are Theme ›
/// Card, a ticket's are Group › Ticket › Agent or Terminal, and ⌘↑ goes to the
/// parent in the same outline.
struct SidebarOutlinePathTests {
    /// The ticket group holding the ticket for `task`, as the sidebar draws it.
    private func group(of task: String, in outline: SidebarOutline) throws -> OneTreeNode {
        try #require(outline.tickets.first { $0.children.contains { $0.id == "ticket:\(task)" } })
    }

    @Test("A card's crumbs are its theme then the card; the old tree would have added the ticket under it")
    func cardCrumbs() throws {
        let outline = try SidebarOutlineTests.outline(filter: .all)
        let crumbs = outline.crumbs(to: .task("t2"))
        // The miniature plan names no card titles, so a card reads as its key.
        #expect(crumbs.map(\.title).first == "Plan")
        #expect(crumbs.map(\.title).last?.hasPrefix("ov-2") == true)
        #expect(crumbs.count == 2, "no ticket under the card")
        #expect(crumbs.map(\.target) == [.theme("theme-Plan"), .task("t2")])
        // The same card under its second theme, by the hint.
        #expect(outline.crumbs(to: .task("t3"), hint: "theme:theme-Mac/task:t3").map(\.title).first == "Mac")
    }

    @Test("A ticket's crumbs are its ticket group, the ticket, then its terminal; the group goes to the Plan's home")
    func ticketCrumbs() throws {
        let outline = try SidebarOutlineTests.outline(filter: .all)
        let group = try group(of: "t2", in: outline)
        let ticketCrumbs = outline.crumbs(to: .ticket("t2"))
        #expect(ticketCrumbs.count == 2)
        #expect(ticketCrumbs.map(\.title) == ["Building", "ov-2 Title t2"])
        #expect(ticketCrumbs.map(\.title).first == group.title)
        #expect(ticketCrumbs.map(\.target) == [.plan, .ticket("t2")])
        let pane = outline.crumbs(to: .terminal(worktree: "wt-copy", terminal: "c2"))
        #expect(pane.count == 3)
        #expect(pane.map(\.title).first == group.title)
        #expect(pane.map(\.target).dropFirst().first == .ticket("t2"))
        #expect(pane.last?.target == .terminal(worktree: "wt-copy", terminal: "c2"))
    }

    @Test("⌘↑ from a ticket reaches its group, from a terminal its ticket, from a card its theme")
    func goingUp() throws {
        let outline = try SidebarOutlineTests.outline(filter: .all)
        let group = try group(of: "t2", in: outline)
        #expect(group.id == "tickets:building")
        #expect(outline.parent(of: .ticket("t2")) == SidebarOutline.Up(target: .plan, hint: group.id))
        #expect(
            outline.parent(of: .terminal(worktree: "wt-copy", terminal: "c2"))
                == SidebarOutline.Up(target: .ticket("t2"), hint: "ticket:t2"))
        #expect(outline.parent(of: .task("t2")) == SidebarOutline.Up(target: .theme("theme-Plan"), hint: "theme:theme-Plan"))
        #expect(outline.parent(of: .theme("theme-Plan")) == SidebarOutline.Up(target: .plan, hint: nil))
        #expect(outline.parent(of: .plan) == nil)
        #expect(outline.parent(of: .task("nowhere")) == nil)
    }

    @Test("A worktree opened whole is its ticket: its crumbs and ⌘↑ are the ticket's")
    func worktreeIsItsTicket() throws {
        let outline = try SidebarOutlineTests.outline(filter: .all)
        #expect(outline.crumbs(to: .worktree("wt-copy")).map(\.target) == [.plan, .ticket("t2")], "the first of the two tickets on it")
        #expect(outline.parent(of: .worktree("wt-copy"))?.hint == (try group(of: "t2", in: outline)).id)
    }
}

extension SidebarOutlinePathTests {
    @Test("A row's siblings are in its own list: a ticket group's are the other groups, never the themes")
    func siblingsStayInTheirList() throws {
        let outline = try SidebarOutlineTests.outline(filter: .all)
        let group = try #require(outline.tickets.first)
        #expect(outline.siblings(of: group.id).map(\.id) == outline.tickets.map(\.id))
        let theme = try #require(outline.themes.first)
        #expect(outline.siblings(of: theme.id).map(\.id) == outline.themes.map(\.id))
        let ticket = try #require(group.children.first)
        #expect(outline.siblings(of: ticket.id).map(\.id) == group.children.map(\.id))
        #expect(outline.siblings(of: "nowhere").isEmpty)
    }
}
