import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A board with no task at all says what tasks are for, never "You're all
/// caught up." (ov-205); one with tasks and nothing unread still does.
@MainActor
@Suite(.serialized)
struct BoardBlankStateTests {
    /// A board read through a stubbed CLI that lists `count` tasks.
    private static func drawn(tasks count: Int, markRead: Bool) async -> Set<String> {
        let defaults = UserDefaults(suiteName: "ov205-\(UUID().uuidString)")!
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        client.commandRunnerForTesting = { args in
            guard args.starts(with: ["task", "list"]) else { return (Data(), nil) }
            let rows = (1...max(count, 1)).prefix(count).map { n in
                let at = now - Int64(n) * 100_000
                return #"{"id":"t\#(n)","key":"bil-\#(n)","title":"Task \#(n)","status":"todo","status_since":\#(at),"created_at":\#(at),"updated_at":\#(at)}"#
            }
            return (Data(#"{"tasks":[\#(rows.joined(separator: ","))]}"#.utf8), nil)
        }
        let store = TaskBoardStore(
            client: client, workspace: .implicit(repository: "r"), readStore: DefaultsBoardReads(defaults))
        await store.readIfNeverRead()
        if markRead { for row in store.board.rows { store.markRead(row) } }

        let seen = NavigatorFilterTests.Seen()
        let host = NSHostingView(
            rootView: NavigatorFilterTests.Hosted(
                level: NavigatorFilterTests.Level(), store: store, defaults: defaults, seen: seen, slowdown: 1,
                confirmation: AskedToMarkRead().confirmation, onStep: { _ in }))
        let window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 300, height: 900),
            styleMask: [.borderless], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        for _ in 0..<15 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        window.close()
        return Set(seen.views.keys)
    }

    @Test("A board with no task shows the empty-board state, not Unread's caught-up line")
    func zeroTasks() async {
        let drawn = await Self.drawn(tasks: 0, markRead: false)
        #expect(drawn.contains("board-empty"), "\(drawn)")
        #expect(!drawn.contains("board-summary-empty"), "\(drawn)")
    }

    @Test("A board with tasks and nothing unread still says it's all caught up")
    func tasksAllRead() async {
        let drawn = await Self.drawn(tasks: 2, markRead: true)
        #expect(drawn.contains("board-summary-empty"), "\(drawn)")
        #expect(!drawn.contains("board-empty"), "\(drawn)")
    }

    @Test("The empty-board state is a lead line and rows, and a blank board plans no Unread")
    func copyAndPlan() {
        EmptyStateCopyTests.expectScannable(BoardBlankState.copy)
        let blank = NavigatorFiltering.make(
            filter: "", board: TaskBoardModel(columns: []), hasOrchestrator: true, agent: nil, unreadMatches: false,
            worktrees: BoardWorktrees())
        #expect(blank.isBlank && !blank.showsUnread)
    }
}
