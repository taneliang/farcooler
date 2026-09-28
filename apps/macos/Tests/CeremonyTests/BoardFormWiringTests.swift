import AgentKit
import AppKit
import Foundation
import SwiftUI
import Testing

@testable import Far_Cooler

/// The board picks its form from its OWN width (owner decision 3, spec §5).
///
/// `BoardForm.resolve` is AgentKit's and tested there; what only a drawn
/// board can show is which width it is handed. The board shares its window
/// with a sidebar and other columns, so a board that measured the window
/// would draw a kanban into a 600 pt pane of a wide window, one column and a
/// half wide. So this hosts the real `TaskBoardView` in a 600 pt frame
/// inside a 1600 pt host and reads which form it drew.
///
/// Read from `BoardFormPreference`, which the board publishes from the same
/// value it switches on, and not from the accessibility identifiers the
/// plan named (`board-list`, `board-kanban`, which are still set): an
/// unshown `NSHostingView` answers no accessibility children at all, so a
/// search for them finds nothing either way.
@MainActor
struct BoardFormWiringTests {
    /// A board with one task on it, read from a stub rather than a runner.
    private static func store() -> TaskBoardStore {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in
            if args.starts(with: ["task", "list"]) {
                return (
                    Data(#"{"tasks":[{"id":"t1","key":"-1","title":"T","status":"todo"}]}"#.utf8),
                    nil
                )
            }
            return (Data(), nil)
        }
        return TaskBoardStore(client: client, workspace: .implicit(repository: "r"))
    }

    /// Every form the board published, in order, and the board's width,
    /// which the test moves.
    @MainActor
    final class Seen: ObservableObject {
        var forms: [BoardForm] = []
        @Published var width: CGFloat
        init(width: CGFloat) { self.width = width }
    }

    private struct Sized: View {
        let store: TaskBoardStore
        let defaults: UserDefaults
        @ObservedObject var seen: Seen
        let host: CGFloat

        var body: some View {
            TaskBoardView(
                store: store, client: store.client, agents: .none, onGoTo: { _ in },
                defaults: defaults
            )
            .frame(width: seen.width, height: 500)
            .frame(width: host, height: 500, alignment: .leading)
            .onPreferenceChange(BoardFormPreference.self) { form in
                MainActor.assumeIsolated {
                    if let form, form != seen.forms.last { seen.forms.append(form) }
                }
            }
        }
    }

    /// A board hosted `host` points wide, drawn at each of `widths` in turn;
    /// the forms it published, in order. `stored` is a choice this device
    /// kept for the board before it was drawn, as after a relaunch.
    private static func forms(
        widths: [CGFloat], host width: CGFloat, stored: BoardForm.Choice = .auto
    ) async -> [BoardForm] {
        let suite = "BoardFormWiringTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = store()
        stored.write(host: store.hostKey, workspace: store.workspace.id, in: defaults)
        await store.readIfNeverRead()
        let seen = Seen(width: widths[0])
        let host = NSHostingView(
            rootView: Sized(store: store, defaults: defaults, seen: seen, host: width))
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: width, height: 500),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        defer { window.close() }
        for board in widths {
            seen.width = board
            for _ in 0..<5 {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
        return seen.forms
    }

    @Test("A 600-pt board in a 1600-pt window draws the list")
    func aNarrowBoardInAWideWindowDrawsTheList() async {
        #expect(await Self.forms(widths: [600], host: 1600) == [.list])
        // And a board as wide as its wide window is a kanban, so the line
        // above can't pass for a board that only ever draws a list.
        #expect(await Self.forms(widths: [1600], host: 1600) == [.kanban])
    }

    /// The hysteresis, drawn: a kanban narrowed to 880 stays a kanban, and
    /// a list widened back to 880 stays a list, so a divider dragged back
    /// and forth inside the band switches nothing. Only 860 does, once.
    /// (A board without the band would draw a list at the first 880 and a
    /// kanban again at 900: four switches.)
    @Test("A board dragged across the line switches once each way")
    func aBoardDraggedAcrossTheLineSwitchesOnceEachWay() async {
        #expect(
            await Self.forms(widths: [900, 880, 900, 860, 880, 860], host: 1600)
                == [.kanban, .list])
    }

    /// **A stored choice is honored on relaunch, from the first frame.** A
    /// board forced to a kanban and drawn 600 pt wide never publishes the
    /// list it would be by width, not even for the frame before its choice
    /// was read.
    @Test("A stored choice is honored on relaunch, from the first frame")
    func aStoredChoiceIsHonoredOnRelaunch() async {
        #expect(await Self.forms(widths: [600], host: 1600, stored: .kanban) == [.kanban])
        #expect(await Self.forms(widths: [1600], host: 1600, stored: .list) == [.list])
    }
}
