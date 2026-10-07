import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The One tree's three areas, each scrolling on its own (ov-335): the
/// places, the plan's tree, the shells. The rule over the heights is
/// `NavigatorSplit.viewports`, with a cap on the two short areas.
struct OneTreeSplitRuleTests {
    typealias Pane = NavigatorSplit.Pane

    private static func panes(places: CGFloat, tree: CGFloat, shells: CGFloat) -> [Pane] {
        [
            Pane(id: "places", content: places, maxShare: OneTreeNavigator.placesShare),
            Pane(id: "tree", fills: true, content: tree),
            Pane(id: "shells", content: shells, maxShare: NavigatorSplit.capShare),
        ]
    }

    @Test("With room for everything, every area is as tall as its rows and none scrolls")
    func roomForAll() {
        let heights = NavigatorSplit.viewports(Self.panes(places: 56, tree: 400, shells: 56), room: 900)
        #expect(heights == ["places": 56, "tree": 400, "shells": 56])
    }

    @Test("A long tree takes what the short areas leave, and the shells stop at a third of the room, then scroll")
    func longTreeAndShells() {
        let room: CGFloat = 800
        let heights = NavigatorSplit.viewports(Self.panes(places: 56, tree: 3000, shells: 2000), room: room)
        #expect(heights["places"] == 56)
        #expect(abs((heights["shells"] ?? 0) - room * NavigatorSplit.capShare) < 0.5, "\(heights)")
        #expect(abs((heights.values.reduce(0, +)) - room) < 0.5, "the tree fills the rest: \(heights)")
        #expect((heights["tree"] ?? 0) > (heights["shells"] ?? 0))
    }

    @Test("A short tree leaves the shells whole: they grow to their rows before they scroll")
    func shortTreeLongShells() {
        let room: CGFloat = 800
        let heights = NavigatorSplit.viewports(Self.panes(places: 56, tree: 60, shells: 900), room: room)
        #expect(heights["tree"] == 60)
        #expect(heights["places"] == 56)
        // Everything else is the shells': 800 - 60 - 56, well past a third.
        #expect(abs((heights["shells"] ?? 0) - 684) < 0.5, "\(heights)")
        // With no room to spare, the shells never scroll beside blank space.
        #expect(abs(heights.values.reduce(0, +) - room) < 0.5, "\(heights)")
    }

    @Test("A middling tree takes its rows, and the shells the rest, between a third and everything")
    func middlingTree() {
        let heights = NavigatorSplit.viewports(Self.panes(places: 56, tree: 400, shells: 900), room: 800)
        #expect(heights["tree"] == 400)
        #expect(abs((heights["shells"] ?? 0) - 344) < 0.5, "\(heights)")
    }

    @Test("The places never take more than half the room, and the short shells keep all their rows")
    func tallPlaces() {
        let room: CGFloat = 600
        let heights = NavigatorSplit.viewports(Self.panes(places: 900, tree: 3000, shells: 56), room: room)
        #expect((heights["places"] ?? 0) <= room * OneTreeNavigator.placesShare + 0.5, "\(heights)")
        #expect((heights["places"] ?? 0) > 56, "\(heights)")
        #expect(heights["shells"] == 56)
        #expect(abs(heights.values.reduce(0, +) - room) < 0.5, "\(heights)")
    }

    @Test("A window too short for all of it squeezes them, never past the room: the container itself never scrolls")
    func neverPastTheRoom() {
        for room in stride(from: CGFloat(40), through: 700, by: 60) {
            let heights = NavigatorSplit.viewports(Self.panes(places: 120, tree: 3000, shells: 500), room: room)
            #expect(heights.values.reduce(0, +) <= room + 0.5, "room \(room): \(heights)")
        }
    }

    @Test("A cap never takes an area under its floor of two rows, or below all of its rows if it has fewer")
    func capKeepsTheFloor() {
        let heights = NavigatorSplit.viewports(Self.panes(places: 56, tree: 3000, shells: 56), room: 400)
        #expect((heights["shells"] ?? 0) >= min(56, NavigatorSplit.minimum) - 0.5, "\(heights)")
    }
}

/// The same, in a real window at a short and a tall height.
@MainActor
@Suite(.serialized)
struct OneTreeSplitWindowTests {
    static let tasks = (1...40).map { OneTreeTask(id: "t\($0)", key: "ov-\($0)", title: "Task \($0)", status: .inProgress) }

    static func sidebar(tasks count: Int, key: String = UUID().uuidString) -> OneTreeSidebar {
        let input = OneTreeInput(
            tasks: Array(tasks.prefix(count)),
            worktrees: [OneTreeWorktree(id: "w", name: "spike")],
            mainCheckout: OneTreeWorktree(id: "m", name: "repo", isMainCheckout: true),
            needsYouCount: 1)
        return OneTreeSidebar(tree: { _ in OneTree.build(input) }, selected: nil, hint: nil, key: key, onChoose: { _ in })
    }

    /// Every frame each probe was drawn at, from the first pass.
    static func draw(height: CGFloat, tasks: Int) async -> FirstLayoutHeightTests.Seen {
        let seen = FirstLayoutHeightTests.Seen()
        let size = CGSize(width: 300, height: height)
        let host = FirstLayoutHeightTests.host(
            OneTreeNavigator(sidebar: sidebar(tasks: tasks), filterText: "", keyed: false), seen: seen, size: size)
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: size.width, height: size.height), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFront(nil)
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(30))
        }
        window.close()
        return seen
    }

    @Test("Short and tall: the places, the tree and the shells are three areas that fit the window, never past it")
    func threeAreas() async throws {
        for height: CGFloat in [260, 420, 900] {
            let seen = await Self.draw(height: height, tasks: 40)
            let places = try #require(seen.frames["navigator-pane-places"], "\(height): \(seen.frames.keys.sorted())")
            let tree = try #require(seen.frames["navigator-pane-tree"])
            let shells = try #require(seen.frames["navigator-pane-shells"])
            // In order, one under the next, none over the window's foot.
            #expect(places.maxY <= tree.minY + 0.5 && tree.maxY <= shells.minY + 0.5, "\(height)")
            #expect(shells.maxY <= height + 0.5, "\(height): shells end at \(shells.maxY)")
            #expect(tree.height > 0 && places.height > 0 && shells.height > 0, "\(height)")
            // The shells never take more than a third (and the places half) of the window.
            #expect(shells.height <= height * NavigatorSplit.capShare + 1, "\(height): shells \(shells.height)")
            #expect(places.height <= height * OneTreeNavigator.placesShare + 1, "\(height): places \(places.height)")
        }
    }

    @Test("A tree with room is as tall as its rows, and nothing scrolls: no gap, no correction on the first pass")
    func roomMeansNoScroll() async throws {
        let seen = await Self.draw(height: 900, tasks: 3)
        let tree = try #require(seen.frames["navigator-pane-tree"])
        // Three cards under No Theme: the filter's row is the header, then four rows.
        let history = try #require(seen.history["navigator-pane-tree"])
        #expect(history.last?.height == tree.height)
        #expect(tree.height < 400, "as tall as its rows, not the window: \(tree.height)")
        // The heights the first pass drew are the ones that stay.
        let first = try #require(history.first)
        #expect(abs(first.height - tree.height) < 0.5, "first \(first.height), settled \(tree.height)")
        let shells = try #require(seen.frames["navigator-pane-shells"])
        // Only the rule's line and the rhythm's room under it (ov-406), not a
        // stretch of blank the tree left over.
        let rule = NavigatorSplit.ruleSlot + NavigatorSplit.headerInset
        #expect(abs(shells.minY - tree.maxY - rule) < 2, "the shells sit a rule and its room under the tree (\(tree.maxY) to \(shells.minY))")
    }

    // MARK: Scrolling stays where the person put it (review 1)

    final class Count: ObservableObject {
        @Published var tasks = 40
    }

    struct Scrolling: View {
        @ObservedObject var count: Count
        let key: String

        var body: some View {
            let input = OneTreeInput(
                tasks: Array(OneTreeSplitWindowTests.tasks.prefix(count.tasks)) + (count.tasks > 40 ? [OneTreeTask(id: "t99", key: "ov-99", title: "New", status: .inProgress)] : []),
                worktrees: [OneTreeWorktree(id: "w", name: "spike")],
                mainCheckout: OneTreeWorktree(id: "m", name: "repo", isMainCheckout: true))
            // The first card is selected, at the top of the tree.
            let sidebar = OneTreeSidebar(
                tree: { _ in OneTree.build(input) }, selected: .task("t1"), hint: nil, key: key, onChoose: { _ in })
            OneTreeNavigator(sidebar: sidebar, filterText: "", keyed: false).frame(width: 300, height: 360)
        }
    }

    /// The scroll view with the most to scroll: the tree's pane.
    static func treeScrollView(in view: NSView) -> NSScrollView? {
        var found: [NSScrollView] = []
        func walk(_ view: NSView) {
            if let scroll = view as? NSScrollView { found.append(scroll) }
            view.subviews.forEach(walk)
        }
        walk(view)
        return found.max { ($0.documentView?.frame.height ?? 0) < ($1.documentView?.frame.height ?? 0) }
    }

    @Test("Rows arriving don't scroll the tree back to the selected row, wherever the person has scrolled it")
    func contentChangesNeverScroll() async throws {
        let count = Count()
        let host = NSHostingView(rootView: Scrolling(count: count, key: UUID().uuidString))
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 300, height: 360), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFront(nil)
        defer { window.close() }
        func settle() async {
            for _ in 0..<12 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(30))
            }
        }
        await settle()
        let scroll = try #require(Self.treeScrollView(in: host))
        let clip = scroll.contentView
        let room = (scroll.documentView?.frame.height ?? 0) - clip.bounds.height
        #expect(room > 300, "the tree has something to scroll: \(room)")
        // The person scrolls down to a theme near the foot.
        clip.scroll(to: NSPoint(x: 0, y: 300))
        scroll.reflectScrolledClipView(clip)
        let before = clip.bounds.origin.y
        #expect(before > 100)
        // A card arrives, and the tree's rows grow.
        count.tasks = 41
        await settle()
        #expect(abs(clip.bounds.origin.y - before) < 1, "scrolled from \(before) to \(clip.bounds.origin.y)")
    }

    // MARK: Rows are the height they're said to be, so the lazy tree needs no estimate

    @Test("Every row is drawn at exactly the height the tree sums for it: a plain row, a keyed card, a count, a caption")
    func rowsAreTheirKnownHeight() async throws {
        let seen = FirstLayoutHeightTests.Seen()
        let size = CGSize(width: 340, height: 1600)
        let input = OneTreeInput(
            tasks: [OneTreeTask(id: "t", key: "ov-1", title: "A card", status: .inProgress, worktreeID: "w")],
            worktrees: [OneTreeWorktree(id: "w", name: "lane", terminals: [OneTreeTerminal(id: "a", title: "claude", isAgent: true)])],
            mainCheckout: OneTreeWorktree(id: "m", name: "repo", isMainCheckout: true, terminals: [OneTreeTerminal(id: "s", title: "zsh", isAgent: false)]),
            unreadable: [OneTreeUnreadable(id: "u", key: "ov-9", title: "Odd", status: "someday")], needsYouCount: 2)
        let tree = OneTree.build(input)
        let key = UUID().uuidString
        let host = FirstLayoutHeightTests.host(
            OneTreeNavigator(
                sidebar: OneTreeSidebar(
                    tree: { _ in tree }, selected: nil, hint: nil, key: key, onChoose: { _ in },
                    fold: TreeFoldRequest()),
                filterText: "", keyed: false), seen: seen, size: size)
        let window = NSWindow(
            contentRect: NSRect(origin: NSPoint(x: -4000, y: -4000), size: size), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.orderFront(nil)
        defer { window.close() }
        for _ in 0..<12 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(30))
        }
        var checked = 0
        var sawCaption = false
        for node in tree.allNodes {
            guard let frame = seen.frames["tree-\(node.id)"] else { continue }
            checked += 1
            sawCaption = sawCaption || !node.caption.isEmpty
            #expect(abs(frame.height - OneTreeRowView.height(for: node)) < 0.5, "\(node.id): \(frame.height)")
        }
        #expect(checked >= 6 && sawCaption, "rows drawn: \(checked), a caption among them: \(sawCaption)")
        // And the tree's pane is their sum, on the first pass: no estimate, no gap.
        let pane = try #require(seen.frames["navigator-pane-tree"])
        let sum = OneTree.rows(tree.tree, expansion: OneTreeExpansion()).reduce(CGFloat(0)) { $0 + OneTreeRowView.height(for: $1.node) }
        #expect(abs(pane.height - (sum + NavigatorSplit.paneInset)) < 1, "pane \(pane.height), rows \(sum)")
        let history = try #require(seen.history["navigator-pane-tree"])
        #expect(abs((history.first?.height ?? 0) - pane.height) < 1, "first pass \(history.first?.height ?? 0), settled \(pane.height)")
    }
}
