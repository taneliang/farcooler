import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Revealing a selection in the tree opens its ancestors for the view only
/// (ov-345, jumpbar-icons review R2-2): a theme the window navigated into
/// must not stay open as if the person opened it. In a real window off
/// every screen; no input is sent.
@MainActor
@Suite(.serialized)
struct OneTreeRevealViewTests {
    final class Pick: ObservableObject {
        @Published var selected: OneTreeTarget?
    }

    /// Two themes with one open card each and no lane: both closed by default.
    static func tree() throws -> OneTree {
        func theme(_ name: String, _ card: String, _ ordinal: Int) -> [String: Any] {
            [
                "id": "theme-\(name)", "short": name, "name": name, "outcome": "", "story": "", "story_at": 0,
                "next": "", "owner_ask": "", "state": "active", "ordinal": ordinal,
                "cards": [["task": card, "key": "ov-\(ordinal)"]],
                "counts": ["backlog": 0, "todo": 1, "needs_decision": 0, "in_progress": 0, "in_review": 0, "done": 0,
                           "cancelled": 0],
            ]
        }
        let object: [String: Any] = [
            "now_ms": 1_000_000, "themes": [theme("Alpha", "t1", 1), theme("Beta", "t2", 2)], "lanes": [],
            "order": [], "cards": [],
        ]
        let plan = try PlanModel.decode(JSONSerialization.data(withJSONObject: object))
        return OneTree.build(
            OneTreeInput(
                tasks: [
                    OneTreeTask(id: "t1", key: "ov-1", title: "One", status: .todo),
                    OneTreeTask(id: "t2", key: "ov-2", title: "Two", status: .todo),
                ], plan: plan))
    }

    struct Hosted: View {
        @ObservedObject var pick: Pick
        let seen: NavigatorFilterTests.Seen
        let key: String
        let tree: OneTree

        var body: some View {
            OneTreeNavigator(
                sidebar: OneTreeSidebar(
                    tree: { _ in tree }, selected: pick.selected, hint: nil, key: key, onChoose: { _ in }),
                filterText: "", keyed: false
            )
            .frame(width: 320, height: 600, alignment: .topLeading)
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

    private static let alphaCard = "tree-theme:theme-Alpha/task:t1"
    private static let betaCard = "tree-theme:theme-Beta/task:t2"

    @Test("A theme opened only to show the selection closes again when the selection moves to another theme")
    func revealedAncestorsAreNotKept() async throws {
        let seen = NavigatorFilterTests.Seen()
        let pick = Pick()
        let host = NSHostingView(
            rootView: Hosted(pick: pick, seen: seen, key: "reveal|\(UUID().uuidString)", tree: try Self.tree()))
        let window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 320, height: 600), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.close() }
        func settle() async {
            for _ in 0..<15 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
        await settle()
        #expect(seen.views[Self.alphaCard] == nil && seen.views[Self.betaCard] == nil, "both themes start closed")
        pick.selected = .task("t1")
        await settle()
        #expect(seen.views[Self.alphaCard] != nil, "Alpha opened to show its card: \(seen.views.keys.sorted())")
        pick.selected = .task("t2")
        await settle()
        #expect(seen.views[Self.betaCard] != nil, "Beta opened to show its card")
        #expect(seen.views[Self.alphaCard] == nil, "Alpha was never the person's choice, so it follows its default: closed")
    }
}
