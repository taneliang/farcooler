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

    @MainActor
    final class Seen {
        var form: BoardForm?
    }

    /// The form a board `board` points wide draws, hosted `host` points wide.
    private static func form(board: CGFloat, host width: CGFloat) async -> BoardForm? {
        let suite = "BoardFormWiringTests.\(UUID().uuidString)"
        let defaults = UserDefaults(suiteName: suite)!
        defer { defaults.removePersistentDomain(forName: suite) }
        let store = store()
        await store.readIfNeverRead()
        let seen = Seen()
        let view = TaskBoardView(
            store: store, client: store.client, agents: .none, onGoTo: { _ in }, defaults: defaults
        )
        .frame(width: board, height: 500)
        .frame(width: width, height: 500, alignment: .leading)
        .onPreferenceChange(BoardFormPreference.self) { form in
            MainActor.assumeIsolated { seen.form = form }
        }
        let host = NSHostingView(rootView: view)
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: width, height: 500),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        defer { window.close() }
        for _ in 0..<50 where seen.form == nil {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return seen.form
    }

    @Test("A 600-pt board in a 1600-pt window draws the list")
    func aNarrowBoardInAWideWindowDrawsTheList() async {
        #expect(await Self.form(board: 600, host: 1600) == .list)
        // And a board as wide as its wide window is a kanban, so the line
        // above can't pass for a board that only ever draws a list.
        #expect(await Self.form(board: 1600, host: 1600) == .kanban)
    }
}
