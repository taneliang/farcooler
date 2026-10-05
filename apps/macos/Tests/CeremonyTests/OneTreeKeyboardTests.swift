import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The one tree from the keyboard (ov-321 review H1), in a real window off
/// every screen: the window's request (⌥⌘2, Esc in the filter) gives it the
/// keyboard, ↓ walks every row without leaving for the chat, → opens a
/// group with nowhere to go, and Return on the row already chosen enters it.
@MainActor
@Suite(.serialized)
struct OneTreeKeyboardTests {
    final class Level: ObservableObject {
        @Published var request = 0
    }

    final class Heard {
        var chosen: [OneTreeTarget] = []
        var entered = 0
    }

    /// A board with no plan: the places, No Theme (closed here, so → has
    /// something to open), the checkout and a loose worktree.
    static func sidebar(_ heard: Heard, selected: OneTreeTarget?) -> OneTreeSidebar {
        let input = OneTreeInput(
            tasks: [OneTreeTask(id: "t1", key: "ov-1", title: "One", status: .inProgress)],
            worktrees: [OneTreeWorktree(id: "w", name: "spike")],
            mainCheckout: OneTreeWorktree(id: "m", name: "repo", isMainCheckout: true))
        return OneTreeSidebar(
            tree: { _ in OneTree.build(input) }, selected: selected, hint: nil, key: "test|\(UUID().uuidString)",
            onChoose: { heard.chosen.append($0.target!) })
    }

    struct Hosted: View {
        @ObservedObject var level: Level
        let sidebar: OneTreeSidebar
        let heard: Heard

        /// A field that has the keyboard first, as the chat or a terminal
        /// would: only the window's request takes it to the tree.
        @State private var text = ""
        @FocusState private var typing: Bool

        var body: some View {
            VStack {
                TextField("Elsewhere", text: $text).focused($typing)
                OneTreeNavigator(
                    sidebar: sidebar, filterText: "", keyed: true, focusRequest: level.request,
                    onEnter: { heard.entered += 1 })
            }
            .frame(width: 300, height: 600)
            .onAppear { typing = true }
        }
    }

    static func key(_ code: UInt16, _ character: Int, in window: NSWindow) {
        let text = String(Character(UnicodeScalar(character)!))
        for type in [NSEvent.EventType.keyDown, .keyUp] {
            window.sendEvent(
                NSEvent.keyEvent(
                    with: type, location: .zero, modifierFlags: [], timestamp: ProcessInfo.processInfo.systemUptime,
                    windowNumber: window.windowNumber, context: nil, characters: text,
                    charactersIgnoringModifiers: text, isARepeat: false, keyCode: code)!)
        }
    }

    static func settle(_ host: NSView) async {
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
    }

    func window(selected: OneTreeTarget?) async -> (NSWindow, Level, Heard) {
        let heard = Heard(), level = Level()
        let host = NSHostingView(rootView: Hosted(level: level, sidebar: Self.sidebar(heard, selected: selected), heard: heard))
        let window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 300, height: 600), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        await Self.settle(host)
        return (window, level, heard)
    }

    @Test("The window's request gives the tree the keyboard; ↓ chooses rows, and ↑ stops at Needs You")
    func requestThenArrows() async throws {
        let (window, level, heard) = await window(selected: .plan)
        defer { window.close() }
        level.request += 1
        await Self.settle(window.contentView!)
        // ↓ from the plan's row: No Theme, a group with nowhere to go, takes
        // the cursor and chooses nothing.
        Self.key(125, NSDownArrowFunctionKey, in: window)
        await Self.settle(window.contentView!)
        #expect(heard.chosen.isEmpty)
        // ↑ twice: the plan, then Needs You, the top row: there's no
        // Orchestrator row above it (R-14), and a third ↑ holds at the top.
        Self.key(126, NSUpArrowFunctionKey, in: window)
        Self.key(126, NSUpArrowFunctionKey, in: window)
        await Self.settle(window.contentView!)
        Self.key(126, NSUpArrowFunctionKey, in: window)
        await Self.settle(window.contentView!)
        #expect(heard.chosen == [.plan, .needsYou])
    }

    @Test("Return on the row already chosen enters it, as ⌥⌘3 does")
    func returnEnters() async throws {
        let (window, level, heard) = await window(selected: .plan)
        defer { window.close() }
        level.request += 1
        await Self.settle(window.contentView!)
        Self.key(36, 0x0D, in: window)
        await Self.settle(window.contentView!)
        #expect(heard.entered == 1)
        #expect(heard.chosen.isEmpty)
    }
}
