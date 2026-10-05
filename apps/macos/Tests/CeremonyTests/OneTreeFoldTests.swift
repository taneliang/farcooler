import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Collapse All and Expand All (ov-334), in a real window off every screen:
/// the toggle beside the filter, and the menu's request.
@MainActor
@Suite(.serialized)
struct OneTreeFoldTests {
    final class Fold: ObservableObject {
        @Published var request = TreeFoldRequest()
    }

    struct Hosted: View {
        @ObservedObject var fold: Fold
        let seen: NavigatorFilterTests.Seen

        /// A card with a lane, under No Theme, which is open on a board with no plan.
        private var sidebar: OneTreeSidebar {
            let input = OneTreeInput(
                tasks: [OneTreeTask(id: "t1", key: "ov-1", title: "One", status: .inProgress, worktreeID: "w")],
                worktrees: [OneTreeWorktree(id: "w", name: "spike", terminals: [OneTreeTerminal(id: "s", title: "zsh", isAgent: false)])])
            return OneTreeSidebar(
                tree: { _ in OneTree.build(input) }, selected: nil, hint: nil, key: "fold|\(key)", onChoose: { _ in },
                fold: fold.request)
        }

        let key = UUID().uuidString

        var body: some View {
            OneTreeNavigator(sidebar: sidebar, filterText: "", keyed: false)
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

    @MainActor
    final class Drawn {
        let seen = NavigatorFilterTests.Seen()
        let fold = Fold()
        var host: NSHostingView<Hosted>!
        var window: NavigatorFilterTests.KeyWindow!

        var ids: Set<String> { Set(seen.views.keys) }

        func settle() async {
            for _ in 0..<15 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }

        func press(_ id: String) -> Bool {
            guard let frame = seen.views[id] else { return false }
            let at = NSPoint(x: frame.midX, y: 600 - frame.midY)
            for type in [NSEvent.EventType.leftMouseDown, .leftMouseUp] {
                window.sendEvent(
                    NSEvent.mouseEvent(
                        with: type, location: at, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                        windowNumber: window.windowNumber, context: nil, eventNumber: 0, clickCount: 1, pressure: 1)!)
            }
            return true
        }
    }

    static func draw() async -> Drawn {
        let drawn = Drawn()
        drawn.host = NSHostingView(rootView: Hosted(fold: drawn.fold, seen: drawn.seen))
        drawn.window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 320, height: 600), styleMask: [.borderless],
            backing: .buffered, defer: false)
        drawn.window.isReleasedWhenClosed = false
        drawn.window.contentView = drawn.host
        drawn.window.makeKeyAndOrderFront(nil)
        await drawn.settle()
        return drawn
    }

    private static let card = "tree-group:no-theme/task:t1"
    private static let lane = "tree-group:no-theme/task:t1/worktree:w"

    @Test("The toggle beside the filter collapses the whole tree, then expands all of it")
    func toggleButton() async {
        let drawn = await Self.draw()
        defer { drawn.window.close() }
        #expect(drawn.ids.contains("one-tree-fold") && drawn.ids.contains("one-tree-filter"))
        #expect(drawn.ids.contains(Self.card), "No Theme starts open on a board with no plan: \(drawn.ids)")
        #expect(drawn.press("one-tree-fold"))
        await drawn.settle()
        #expect(!drawn.ids.contains(Self.card), "Collapse All closed No Theme")
        #expect(drawn.press("one-tree-fold"))
        await drawn.settle()
        #expect(drawn.ids.contains(Self.card) && drawn.ids.contains(Self.lane), "Expand All opened the card to its lane")
        #expect(drawn.ids.contains("tree-group:no-theme/task:t1/worktree:w/terminal:s"), "and the lane to its terminal")
    }

    @Test("The menu's request folds the tree, and asking the same twice is two asks")
    func menuRequest() async {
        let drawn = await Self.draw()
        defer { drawn.window.close() }
        drawn.fold.request.collapse()
        await drawn.settle()
        #expect(!drawn.ids.contains(Self.card))
        drawn.fold.request.expand()
        await drawn.settle()
        #expect(drawn.ids.contains(Self.lane))
        drawn.fold.request.collapse()
        await drawn.settle()
        #expect(!drawn.ids.contains(Self.card))
    }

    @Test("The button says what a click does, in words and in a symbol")
    func buttonWords() {
        #expect(TreeFold.title(anyExpanded: true) == "Collapse All")
        #expect(TreeFold.title(anyExpanded: false) == "Expand All")
        #expect(TreeFold.symbol(anyExpanded: true) != TreeFold.symbol(anyExpanded: false))
        #expect(TreeFold.help(anyExpanded: true).hasPrefix("Collapse"))
    }

    @Test("The menu's two commands ask the tree to fold the way they say, and no other command asks anything")
    func commandsFold() {
        var fold = TreeFoldRequest()
        fold.apply(.collapseTree)
        #expect(fold == TreeFoldRequest(serial: 1, expands: false))
        fold.apply(.expandTree)
        #expect(fold == TreeFoldRequest(serial: 2, expands: true))
        let before = fold
        for other in [AppCommand.showPlan, .focusConversation, .boardView, .goUp, .toggleSidebar] { fold.apply(other) }
        #expect(fold == before)
    }

    @Test("An ⌥-click on a disclosure folds the siblings; the keyboard, VoiceOver and a plain click never do")
    func optionClickReadsTheClick() throws {
        let tree = try JumpBarGlyphTests.tree().tree
        let plan = try #require(tree.tree.first { $0.kind == .theme && $0.hasChildren })
        let siblings = tree.siblings(of: plan.id)
        func mouse(_ flags: NSEvent.ModifierFlags, _ type: NSEvent.EventType = .leftMouseDown) -> NSEvent {
            NSEvent.mouseEvent(
                with: type, location: .zero, modifierFlags: flags, timestamp: 0, windowNumber: 0, context: nil,
                eventNumber: 0, clickCount: 1, pressure: 1)!
        }
        let key = NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [.option], timestamp: 0, windowNumber: 0, context: nil,
            characters: " ", charactersIgnoringModifiers: " ", isARepeat: false, keyCode: 49)!
        var closed = OneTreeExpansion()
        closed.setAll(false, in: tree.roots)
        let open = TreeFold.toggled(closed, plan, siblings: siblings, event: mouse([.option]))
        for sibling in siblings where sibling.hasChildren { #expect(open.isExpanded(sibling), "\(sibling.id)") }
        // The same toggle with a key held, a plain click, or no event: the row alone.
        for event in [key, mouse([]), nil] as [NSEvent?] {
            let one = TreeFold.toggled(closed, plan, siblings: siblings, event: event)
            #expect(one.isExpanded(plan))
            #expect(siblings.filter { $0.id != plan.id && $0.hasChildren }.allSatisfy { !one.isExpanded($0) })
        }
        #expect(TreeFold.togglesSiblings(event: mouse([.option], .leftMouseUp)))
        #expect(!TreeFold.togglesSiblings(event: mouse([.option], .rightMouseDown)))
    }

    @Test("Collapse All and Expand All are enabled only where the tree is on screen")
    func menuNeedsTheTree() {
        #expect(MainWindowFocus.treeOnScreen(hasBoard: true, boardView: false, navigatorHidden: false, focused: false))
        #expect(!MainWindowFocus.treeOnScreen(hasBoard: false, boardView: false, navigatorHidden: false, focused: false))
        #expect(!MainWindowFocus.treeOnScreen(hasBoard: true, boardView: true, navigatorHidden: false, focused: false))
        #expect(!MainWindowFocus.treeOnScreen(hasBoard: true, boardView: false, navigatorHidden: true, focused: false))
        #expect(!MainWindowFocus.treeOnScreen(hasBoard: true, boardView: false, navigatorHidden: false, focused: true))
        var focus = MainWindowFocus(overlayOpen: false)
        focus.inWorkspace = true
        #expect(!MainWindowFocus.goes(\.foldsTree, focus))
        focus.foldsTree = true
        #expect(MainWindowFocus.goes(\.foldsTree, focus))
    }
}
