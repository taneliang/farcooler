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
        #expect(abs(shells.minY - tree.maxY) < 2, "the shells meet the tree's rule: no gap (\(tree.maxY) to \(shells.minY))")
    }
}
