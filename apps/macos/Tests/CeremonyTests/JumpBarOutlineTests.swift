import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The jump bar's crumbs, its menus and ⌘↑ walk the sidebar's outline, not
/// `OneTree` (ov-512): a card's crumbs are Theme › Card, a lane's are Group ›
/// Lane › Agent or Terminal, ⌘↑ goes to the parent in that outline, and each
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
        case .lane(let id): .workspace(host: "h", workspace: "w", focus: .plan(.lane(id)))
        case .theme(let id): .workspace(host: "h", workspace: "w", focus: .plan(.theme(id)))
        case .task(let id): .workspace(host: "h", workspace: "w", focus: .task(id))
        case .terminal(let wt, let id): .workspace(host: "h", workspace: "w", focus: .worktree(wt, terminal: id))
        default: nil
        }
    }

    private struct Fixture {
        var outline: SidebarOutline
        var group: OneTreeNode
        var lane: OneTreeNode
        var laneID: String
        var terminal: OneTreeNode
        var terminalTarget: OneTreeTarget
        var card: OneTreeNode
        var theme: OneTreeNode
    }

    private func fixture() throws -> Fixture {
        let outline = try JumpBarGlyphTests.tree().tree
        let group = try #require(
            outline.lanes.first { group in group.children.contains { $0.kind == .lane && $0.children.contains { $0.kind == .terminal } } })
        let lane = try #require(group.children.first { $0.kind == .lane && $0.children.contains { $0.kind == .terminal } })
        guard case .lane(let laneID)? = lane.target else { throw FixtureError.noLane }
        let terminal = try #require(lane.children.first { $0.kind == .terminal && $0.target != nil })
        let theme = try #require(outline.themes.first { $0.kind == .theme && $0.children.contains { $0.kind == .task } })
        let card = try #require(theme.children.first { $0.kind == .task })
        return Fixture(
            outline: outline, group: group, lane: lane, laneID: laneID, terminal: terminal,
            terminalTarget: try #require(terminal.target), card: card, theme: theme)
    }

    private enum FixtureError: Error { case noLane }

    @Test("A lane's crumbs are its lane group then the lane; a card's are its theme then the card")
    func cardVersusLane() throws {
        let f = try fixture()
        let lane = try #require(
            ContentView.treePath(to: .lane(f.laneID), in: f.outline, hint: nil, top: top, resolve: resolve))
        #expect(lane.crumbs.map(\.title) == ["Billing", f.group.title, f.lane.title])
        #expect(lane.crumbs.map(\.glyph) == [nil, f.group.glyph, f.lane.glyph])
        // The group is a way to the Plan's home, which holds its section; the lane is where you are.
        #expect(lane.crumbs.map(\.target) == [top.target, home, nil])
        #expect(lane.nodes.map(\.id) == [f.group.id, f.lane.id])

        let cardTarget = try #require(f.card.target)
        let card = try #require(
            ContentView.treePath(to: cardTarget, in: f.outline, hint: f.card.id, top: top, resolve: resolve))
        #expect(card.crumbs.map(\.title) == ["Billing", f.theme.title, "\(f.card.key) \(f.card.title)"])
        #expect(card.crumbs.map(\.target) == [top.target, resolve(try #require(f.theme.target)), nil])
        // No lane under a card: the old path's Theme › Task › Lane › Terminal.
        #expect(card.nodes.map(\.kind) == [.theme, .task])
    }

    @Test("An agent or terminal pane's crumbs are group, lane, pane")
    func paneCrumbs() throws {
        let f = try fixture()
        let path = try #require(
            ContentView.treePath(to: f.terminalTarget, in: f.outline, hint: f.terminal.id, top: top, resolve: resolve))
        #expect(path.crumbs.map(\.title).dropFirst().first == f.group.title)
        #expect(path.crumbs.count == 4)
        #expect(path.crumbs[2].title == f.lane.title)
        #expect(path.crumbs[3].glyph == f.terminal.glyph)
        #expect(path.crumbs[2].target == resolve(.lane(f.laneID)))
    }

    @Test("⌘↑ from a lane reaches its group; from a pane its lane; from a card its theme")
    func goingUp() throws {
        let f = try fixture()
        let fromLane = try #require(ContentView.treeUp(from: .lane(f.laneID), in: f.outline, hint: nil, resolve: resolve))
        #expect(fromLane.selection == home)
        #expect(fromLane.hint == f.group.id)
        let fromPane = try #require(
            ContentView.treeUp(from: f.terminalTarget, in: f.outline, hint: f.terminal.id, resolve: resolve))
        #expect(fromPane.selection == resolve(.lane(f.laneID)))
        let cardTarget = try #require(f.card.target)
        let fromCard = try #require(ContentView.treeUp(from: cardTarget, in: f.outline, hint: f.card.id, resolve: resolve))
        #expect(fromCard.selection == resolve(try #require(f.theme.target)))
        #expect(ContentView.treeUp(from: .plan, in: f.outline, hint: nil, resolve: resolve) == nil)
    }

    @Test("Each crumb's menu lists the rows beside it in the sidebar, the one you're at checked")
    func menusAreSiblings() throws {
        let f = try fixture()
        let lanes = JumpMenus.siblings(of: f.lane, in: f.outline, resolve: resolve)
        let beside = f.group.children.filter { $0.target != nil }
        #expect(lanes.items.map(\.id) == beside.map { "tree|\($0.id)" })
        #expect(lanes.current?.id == "tree|\(f.lane.id)")
        #expect(lanes.items.allSatisfy { $0.glyph != nil })

        let groups = JumpMenus.siblings(of: f.group, in: f.outline, resolve: resolve)
        #expect(groups.items.map(\.title) == f.outline.lanes.filter { $0.id.hasPrefix("lanes:") }.map(\.title))
        #expect(groups.current?.title == f.group.title)

        let cards = JumpMenus.siblings(of: f.card, in: f.outline, resolve: resolve)
        #expect(cards.items.map(\.id) == f.theme.children.filter { $0.target != nil }.map { "tree|\($0.id)" })
        #expect(cards.current?.id == "tree|\(f.card.id)")
        #expect(cards.items.first { $0.id == "tree|\(f.card.id)" }?.key == f.card.key)
    }
}
