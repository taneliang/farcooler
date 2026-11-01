import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The jump bar's crumbs, its menus and ⌘↑ walk the sidebar's outline, not
/// `OneTree` (ov-512): a card's crumbs are Theme › Card, a ticket's are Group ›
/// Ticket › Agent or Terminal, ⌘↑ goes to the parent in that outline, and each
/// crumb's menu lists that level's siblings. Over the real plan the CLI wrote
/// (`JumpBarGlyphTests.tree`), through the window's own pure steps.
@MainActor
struct JumpBarOutlineTests {
    typealias Selection = ContentView.Selection

    private let home = Selection.workspace(host: "h", workspace: "w", focus: nil)
    private let top = WorkspaceNavigation.Crumb(
        title: "Billing", target: .workspace(host: "h", workspace: "w", focus: nil))

    /// Where a target is, for these tests: the Plan's home for the plan, else
    /// a place named by the target itself.
    private func resolve(_ target: OneTreeTarget) -> Selection? {
        switch target {
        case .plan: home
        case .ticket(let id): .workspace(host: "h", workspace: "w", focus: .plan(.ticket(id)))
        case .theme(let id): .workspace(host: "h", workspace: "w", focus: .plan(.theme(id)))
        case .task(let id): .workspace(host: "h", workspace: "w", focus: .task(id))
        case .terminal(let wt, let id): .workspace(host: "h", workspace: "w", focus: .worktree(wt, terminal: id))
        default: nil
        }
    }

    private struct Fixture {
        var outline: SidebarOutline
        var group: OneTreeNode
        var ticket: OneTreeNode
        var ticketID: String
        var terminal: OneTreeNode
        var terminalTarget: OneTreeTarget
        var card: OneTreeNode
        var theme: OneTreeNode
    }

    private func fixture() throws -> Fixture {
        let outline = try JumpBarGlyphTests.tree().tree
        let group = try #require(
            outline.tickets.first { group in group.children.contains { $0.kind == .ticket && $0.children.contains { $0.kind == .terminal } } })
        let ticket = try #require(group.children.first { $0.kind == .ticket && $0.children.contains { $0.kind == .terminal } })
        guard case .ticket(let ticketID)? = ticket.target else { throw FixtureError.noTicket }
        let terminal = try #require(ticket.children.first { $0.kind == .terminal && $0.target != nil })
        let theme = try #require(outline.themes.first { $0.kind == .theme && $0.children.contains { $0.kind == .task } })
        let card = try #require(theme.children.first { $0.kind == .task })
        return Fixture(
            outline: outline, group: group, ticket: ticket, ticketID: ticketID, terminal: terminal,
            terminalTarget: try #require(terminal.target), card: card, theme: theme)
    }

    private enum FixtureError: Error { case noTicket }

    @Test("A ticket's crumbs are its group then the ticket; a card's are its theme then the card")
    func cardVersusTicket() throws {
        let f = try fixture()
        let ticket = try #require(
            ContentView.treePath(to: .ticket(f.ticketID), in: f.outline, hint: nil, top: top, resolve: resolve))
        #expect(ticket.crumbs.map(\.title) == ["Billing", f.group.title, "\(f.ticket.key) \(f.ticket.title)"])
        #expect(ticket.crumbs.map(\.glyph) == [nil, f.group.glyph, f.ticket.glyph])
        // The group is a way to the Plan's home, which holds its section; the ticket is where you are.
        #expect(ticket.crumbs.map(\.target) == [top.target, home, nil])
        #expect(ticket.nodes.map(\.id) == [f.group.id, f.ticket.id])

        let cardTarget = try #require(f.card.target)
        let card = try #require(
            ContentView.treePath(to: cardTarget, in: f.outline, hint: f.card.id, top: top, resolve: resolve))
        #expect(card.crumbs.map(\.title) == ["Billing", f.theme.title, "\(f.card.key) \(f.card.title)"])
        #expect(card.crumbs.map(\.target) == [top.target, resolve(try #require(f.theme.target)), nil])
        // No ticket under a card: the old path's Theme › Task › Ticket › Terminal.
        #expect(card.nodes.map(\.kind) == [.theme, .task])
    }

    @Test("An agent or terminal pane's crumbs are group, ticket, pane")
    func paneCrumbs() throws {
        let f = try fixture()
        let path = try #require(
            ContentView.treePath(to: f.terminalTarget, in: f.outline, hint: f.terminal.id, top: top, resolve: resolve))
        #expect(path.crumbs.map(\.title).dropFirst().first == f.group.title)
        #expect(path.crumbs.count == 4)
        #expect(path.crumbs[2].title == "\(f.ticket.key) \(f.ticket.title)", "a ticket with a key reads key then title, as a card does")
        #expect(path.crumbs[3].glyph == f.terminal.glyph)
        #expect(path.crumbs[2].target == resolve(.ticket(f.ticketID)))
    }

    @Test("⌘↑ from a ticket reaches its group; from a pane its ticket; from a card its theme")
    func goingUp() throws {
        let f = try fixture()
        let fromTicket = try #require(ContentView.treeUp(from: .ticket(f.ticketID), in: f.outline, hint: nil, resolve: resolve))
        #expect(fromTicket.selection == home)
        #expect(fromTicket.hint == f.group.id)
        let fromPane = try #require(
            ContentView.treeUp(from: f.terminalTarget, in: f.outline, hint: f.terminal.id, resolve: resolve))
        #expect(fromPane.selection == resolve(.ticket(f.ticketID)))
        let cardTarget = try #require(f.card.target)
        let fromCard = try #require(ContentView.treeUp(from: cardTarget, in: f.outline, hint: f.card.id, resolve: resolve))
        #expect(fromCard.selection == resolve(try #require(f.theme.target)))
        #expect(ContentView.treeUp(from: .plan, in: f.outline, hint: nil, resolve: resolve) == nil)
    }

    @Test("Each crumb's menu lists the rows beside it in the sidebar, the one you're at checked")
    func menusAreSiblings() throws {
        let f = try fixture()
        let tickets = JumpMenus.siblings(of: f.ticket, in: f.outline, resolve: resolve)
        let beside = f.group.children.filter { $0.target != nil }
        #expect(tickets.items.map(\.id) == beside.map { "tree|\($0.id)" })
        #expect(tickets.current?.id == "tree|\(f.ticket.id)")
        #expect(tickets.items.allSatisfy { $0.glyph != nil })

        let groups = JumpMenus.siblings(of: f.group, in: f.outline, resolve: resolve)
        #expect(groups.items.map(\.title) == f.outline.tickets.filter { $0.id.hasPrefix("tickets:") }.map(\.title))
        #expect(groups.current?.title == f.group.title)

        let cards = JumpMenus.siblings(of: f.card, in: f.outline, resolve: resolve)
        #expect(cards.items.map(\.id) == f.theme.children.filter { $0.target != nil }.map { "tree|\($0.id)" })
        #expect(cards.current?.id == "tree|\(f.card.id)")
        #expect(cards.items.first { $0.id == "tree|\(f.card.id)" }?.key == f.card.key)
    }
}
