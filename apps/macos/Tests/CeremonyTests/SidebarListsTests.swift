import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// The sidebar as two lists (ov-503 phase 6), in a real window off every
/// screen, no input but the key events a test sends the window itself: the
/// keyboard walks Themes into Lanes, choosing one side lights the other,
/// revealing a pane opens its lane for the view only, and Collapse All and
/// Expand All reach both lists.
@MainActor
@Suite(.serialized)
struct SidebarListsTests {
    final class Pick: ObservableObject {
        @Published var selected: OneTreeTarget?
        @Published var request = 0
        @Published var fold = TreeFoldRequest()
    }

    final class Heard {
        var chosen: [OneTreeTarget] = []
    }

    /// Two themes. The lane `relay` works t1 and t3, one card in each, and
    /// `solo` works t2; both have a worktree with panes.
    static func outline() throws -> SidebarOutline {
        func theme(_ name: String, _ cards: [String], _ ordinal: Int) -> [String: Any] {
            [
                "id": "theme-\(name)", "short": name, "name": name, "outcome": "", "story": "", "story_at": 0,
                "next": "", "owner_ask": "", "state": "active", "ordinal": ordinal,
                "cards": cards.map { ["task": $0, "key": "ov-\($0.dropFirst())"] },
                "counts": ["backlog": 0, "todo": 0, "needs_decision": 0, "in_progress": cards.count, "in_review": 0,
                           "done": 0, "cancelled": 0],
            ]
        }
        func lane(_ name: String, _ state: String, _ cards: [String]) -> [String: Any] {
            [
                "id": "lane-\(name)", "short": name, "name": name, "state": state, "reason": "", "plan_rank": NSNull(),
                "worktree_path": ".claude/worktrees/\(name)", "branch": name, "harness": "claude", "model": "opus",
                "train": NSNull(), "landed_sha": NSNull(), "state_since": 999_000_000, "stale": false, "fix_rounds": 0,
                "cards": cards.map { ["task": $0, "key": "ov-\($0.dropFirst())", "slice": ""] },
                "agents": [], "spend": ["input_tokens": 0, "runs": 0, "unmeasured_agents": 0],
            ]
        }
        let object: [String: Any] = [
            "now_ms": 1_000_000_000,
            "themes": [theme("Alpha", ["t1", "t2"], 1), theme("Beta", ["t3"], 2)],
            "lanes": [lane("relay", "building", ["t1", "t3"]), lane("solo", "review", ["t2"])],
            "order": [], "cards": [],
        ]
        let plan = try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
        func worktree(_ name: String) -> OneTreeWorktree {
            OneTreeWorktree(
                id: "wt-\(name)", name: name, path: "/repo/.claude/worktrees/\(name)", branch: name,
                terminals: [OneTreeTerminal(id: "\(name)-sh", title: "zsh", isAgent: false)])
        }
        return SidebarOutline.build(
            OneTreeInput(
                tasks: ["t1", "t2", "t3"].map { OneTreeTask(id: $0, key: "ov-\($0.dropFirst())", title: "Card \($0)", status: .inProgress) },
                plan: plan, worktrees: [worktree("relay"), worktree("solo")]))
    }

    struct Hosted: View {
        @ObservedObject var pick: Pick
        let seen: NavigatorFilterTests.Seen
        let heard: Heard
        let key: String
        let outline: SidebarOutline

        var body: some View {
            OneTreeNavigator(
                sidebar: OneTreeSidebar(
                    tree: { _ in outline }, selected: pick.selected, hint: nil, key: key,
                    onChoose: { node in
                        if let target = node.target { heard.chosen.append(target) }
                    }, fold: pick.fold),
                filterText: "", keyed: true, focusRequest: pick.request
            )
            .frame(width: 320, height: 900, alignment: .topLeading)
            .environment(\.gridProbing, true)
            .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                GeometryReader { proxy in
                    let _ = seen.views = Dictionary(
                        probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                    Color.clear
                }
            }
        }
    }

    @MainActor
    final class Drawn {
        let seen = NavigatorFilterTests.Seen()
        let pick = Pick()
        let heard = Heard()
        var host: NSHostingView<Hosted>!
        var window: NavigatorFilterTests.KeyWindow!
        var ids: Set<String> { Set(seen.views.keys) }

        func settle() async {
            for _ in 0..<15 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
    }

    static func draw(selected: OneTreeTarget? = nil) async throws -> Drawn {
        let drawn = Drawn()
        drawn.pick.selected = selected
        drawn.host = NSHostingView(
            rootView: Hosted(
                pick: drawn.pick, seen: drawn.seen, heard: drawn.heard, key: "lists|\(UUID().uuidString)",
                outline: try outline()))
        drawn.window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 320, height: 900), styleMask: [.borderless],
            backing: .buffered, defer: false)
        drawn.window.isReleasedWhenClosed = false
        drawn.window.contentView = drawn.host
        TestWindow.show(drawn.window)
        await drawn.settle()
        return drawn
    }

    private static let alphaT1 = "tree-theme:theme-Alpha/task:t1"
    private static let betaT3 = "tree-theme:theme-Beta/task:t3"
    private static let alphaT2 = "tree-theme:theme-Alpha/task:t2"
    private static let relay = "tree-lane:lane-relay"
    private static let solo = "tree-lane:lane-solo"

    @Test("A card lights the lane working it, and a lane lights each of its cards; nothing else is lit")
    func crossHighlight() async throws {
        let drawn = try await Self.draw(selected: .task("t1"))
        defer { drawn.window.close() }
        func lit() -> Set<String> { Set(drawn.ids.filter { $0.hasPrefix("tree-related-") }.map { "tree-" + $0.dropFirst("tree-related-".count) }) }
        #expect(lit() == [Self.relay], "card ov-1 lights its lane: \(drawn.ids.sorted())")
        drawn.pick.selected = .lane("lane-relay")
        await drawn.settle()
        #expect(lit() == [Self.alphaT1, Self.betaT3], "the lane lights both its cards, one in each theme")
        drawn.pick.selected = .lane("lane-solo")
        await drawn.settle()
        #expect(lit() == [Self.alphaT2])
        drawn.pick.selected = .theme("theme-Alpha")
        await drawn.settle()
        #expect(lit().isEmpty, "a theme goes with no lane")
    }

    @Test("↓ walks from the Themes list into the Lanes list and chooses a lane on arrival")
    func keyboardCrossesTheLists() async throws {
        let drawn = try await Self.draw(selected: .task("t3"))
        defer { drawn.window.close() }
        drawn.pick.request += 1
        await drawn.settle()
        for _ in 0..<4 {
            OneTreeKeyboardTests.key(125, NSDownArrowFunctionKey, in: drawn.window)
            await drawn.settle()
        }
        // t3 is the last card; below it the Lanes list: Building (a group, so
        // the cursor stops there without choosing), then the lane relay.
        #expect(drawn.heard.chosen.contains(.lane("lane-relay")), "chosen: \(drawn.heard.chosen)")
        #expect(drawn.heard.chosen.first != .lane("lane-relay"))
    }

    @Test("Revealing a lane's pane opens its lane for the view only, and shuts it when the selection leaves")
    func revealOpensTheLane() async throws {
        let drawn = try await Self.draw()
        defer { drawn.window.close() }
        let pane = "\(Self.relay)/terminal:relay-sh"
        #expect(drawn.ids.contains(Self.relay) && !drawn.ids.contains(pane), "lanes start closed")
        drawn.pick.selected = .terminal(worktree: "wt-relay", terminal: "relay-sh")
        await drawn.settle()
        #expect(drawn.ids.contains(pane), "opened to show the pane: \(drawn.ids.sorted())")
        drawn.pick.selected = .terminal(worktree: "wt-solo", terminal: "solo-sh")
        await drawn.settle()
        #expect(drawn.ids.contains("\(Self.solo)/terminal:solo-sh"))
        #expect(!drawn.ids.contains(pane), "relay was never the person's choice, so it follows its default")
    }

    @Test("Collapse All closes both lists, and Expand All opens the work in them")
    func foldReachesBothLists() async throws {
        let drawn = try await Self.draw()
        defer { drawn.window.close() }
        #expect(drawn.ids.contains(Self.alphaT1) && drawn.ids.contains(Self.relay))
        drawn.pick.fold.collapse()
        await drawn.settle()
        #expect(!drawn.ids.contains(Self.alphaT1), "the themes closed")
        #expect(!drawn.ids.contains(Self.relay), "and the lane groups")
        drawn.pick.fold.expand()
        await drawn.settle()
        #expect(drawn.ids.contains(Self.alphaT1) && drawn.ids.contains(Self.relay))
        #expect(drawn.ids.contains("\(Self.relay)/terminal:relay-sh"), "a lane opens to its panes")
    }
}
