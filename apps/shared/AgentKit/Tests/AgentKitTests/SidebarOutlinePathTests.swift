import Foundation
import Testing

@testable import AgentKit

/// The jump bar's path and ⌘↑ walk the sidebar's outline (ov-512), not
/// `OneTree`'s Theme › Task › Lane › Terminal: a card's crumbs are Theme ›
/// Card, a lane's are Group › Lane › Agent or Terminal, and ⌘↑ goes to the
/// parent in the same outline.
struct SidebarOutlinePathTests {
    private func lane(_ name: String) throws -> String {
        try #require(OneTreeTests.board().plan.lanes.first { $0.name == name }).id
    }

    /// The lane group holding `lane`, as the sidebar draws it.
    private func group(of lane: String, in outline: SidebarOutline) throws -> OneTreeNode {
        try #require(outline.lanes.first { $0.children.contains { $0.id == "lane:\(lane)" } })
    }

    @Test("A card's crumbs are its theme then the card; the old tree would have added the lane under it")
    func cardCrumbs() throws {
        let outline = try SidebarOutlineTests.outline(filter: .all)
        let crumbs = outline.crumbs(to: .task("t2"))
        // The miniature plan names no card titles, so a card reads as its key.
        #expect(crumbs.map(\.title).first == "Plan")
        #expect(crumbs.map(\.title).last?.hasPrefix("ov-2") == true)
        #expect(crumbs.count == 2, "no lane under the card")
        #expect(crumbs.map(\.target) == [.theme("theme-Plan"), .task("t2")])
        // The same card under its second theme, by the hint.
        #expect(outline.crumbs(to: .task("t3"), hint: "theme:theme-Mac/task:t3").map(\.title).first == "Mac")
    }

    @Test("A lane's crumbs are its lane group, the lane, then its terminal; the group goes to the Plan's home")
    func laneCrumbs() throws {
        let outline = try SidebarOutlineTests.outline(filter: .all)
        let copy = try lane("copy")
        let group = try group(of: copy, in: outline)
        let laneCrumbs = outline.crumbs(to: .lane(copy))
        #expect(laneCrumbs.count == 2)
        #expect(laneCrumbs.map(\.title).first == group.title)
        #expect(laneCrumbs.map(\.target) == [.plan, .lane(copy)])
        let pane = outline.crumbs(to: .terminal(worktree: "wt-copy", terminal: "c2"))
        #expect(pane.count == 3)
        #expect(pane.map(\.title).first == group.title)
        #expect(pane.map(\.target).dropFirst().first == .lane(copy))
        #expect(pane.last?.target == .terminal(worktree: "wt-copy", terminal: "c2"))
    }

    @Test("⌘↑ from a lane reaches its group, from a terminal its lane, from a card its theme")
    func goingUp() throws {
        let outline = try SidebarOutlineTests.outline(filter: .all)
        let copy = try lane("copy")
        let group = try group(of: copy, in: outline)
        #expect(group.id.hasPrefix("lanes:"))
        #expect(outline.parent(of: .lane(copy)) == SidebarOutline.Up(target: .plan, hint: group.id))
        #expect(
            outline.parent(of: .terminal(worktree: "wt-copy", terminal: "c2"))
                == SidebarOutline.Up(target: .lane(copy), hint: "lane:\(copy)"))
        #expect(outline.parent(of: .task("t2")) == SidebarOutline.Up(target: .theme("theme-Plan"), hint: "theme:theme-Plan"))
        #expect(outline.parent(of: .theme("theme-Plan")) == SidebarOutline.Up(target: .plan, hint: nil))
        #expect(outline.parent(of: .plan) == nil)
        #expect(outline.parent(of: .task("nowhere")) == nil)
    }

    @Test("A worktree opened whole is its lane: its crumbs and ⌘↑ are the lane's")
    func worktreeIsItsLane() throws {
        let outline = try SidebarOutlineTests.outline(filter: .all)
        let copy = try lane("copy")
        #expect(outline.crumbs(to: .worktree("wt-copy")).map(\.target) == [.plan, .lane(copy)])
        #expect(outline.parent(of: .worktree("wt-copy"))?.hint == (try group(of: copy, in: outline)).id)
    }
}

extension SidebarOutlinePathTests {
    @Test("A row's siblings are in its own list: a lane group's are the other groups, never the themes")
    func siblingsStayInTheirList() throws {
        let outline = try SidebarOutlineTests.outline(filter: .all)
        let group = try #require(outline.lanes.first)
        #expect(outline.siblings(of: group.id).map(\.id) == outline.lanes.map(\.id))
        let theme = try #require(outline.themes.first)
        #expect(outline.siblings(of: theme.id).map(\.id) == outline.themes.map(\.id))
        let lane = try #require(group.children.first)
        #expect(outline.siblings(of: lane.id).map(\.id) == group.children.map(\.id))
        #expect(outline.siblings(of: "nowhere").isEmpty)
    }
}
