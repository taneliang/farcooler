import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Quitting with two windows open and launching again puts both back where
/// they were (ov-262), through what the app itself runs: the window store,
/// the app delegate's quit, the windows taking their records, and each
/// window's restore task in a real window.
///
/// Every window of the owner's relaunch opened on "No Workspace Selected":
/// SwiftUI cancelled each window's restore task once as it came up and
/// started it again, and the cancelled run cleared the open it hadn't
/// finished, so the run that followed had nothing to open.
@MainActor
struct WindowRestoreTests {
    typealias World = DestinationResolver.World
    private static let billing = "billing", shop = "shop"

    private static func workspace(_ id: String) -> Destination {
        Destination(runner: .init(host: ""), place: .workspace(id))
    }

    /// One window's half of `ContentView`: its open, its selection, and the
    /// restore task, mounted while `mounted`.
    @MainActor @Observable final class Window {
        var restoring: DestinationOpen?
        var selection: ContentView.Selection?
        var mounted = true
        /// This Mac's runner, still coming up until a test says it's up.
        var world = World(seats: [World.Seat(host: "", ready: false)])
    }

    struct Host: View {
        @Bindable var window: Window

        var body: some View {
            if window.mounted {
                Color.clear.modifier(
                    WindowRestore(
                        restoring: $window.restoring, interrupted: { window.selection != nil },
                        world: { window.world }, read: { _, _ in nil },
                        land: { window.selection = MacDestination.landing($0, click: false, in: .empty).selection }))
            }
        }
    }

    private static func show(_ window: Window) -> NSWindow {
        let shown = NSWindow(
            contentRect: NSRect(x: -6000, y: -6000, width: 200, height: 120), styleMask: [.titled],
            backing: .buffered, defer: false)
        shown.isReleasedWhenClosed = false
        shown.contentView = NSHostingView(rootView: Host(window: window))
        shown.orderFrontRegardless()
        return shown
    }

    private static func settle(_ windows: [NSWindow], for duration: Duration = .milliseconds(300)) async throws {
        let end = ContinuousClock.now + duration
        while ContinuousClock.now < end {
            for window in windows { window.contentView?.layoutSubtreeIfNeeded() }
            try await Task.sleep(for: .milliseconds(20))
        }
    }

    @Test("Two windows open at a quit come back where they were, though each restore task is cancelled once")
    func twoWindowsComeBack() async throws {
        let suite = "farcooler.test.restore.twoWindows"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defaults.removePersistentDomain(forName: suite)
        defer { defaults.removePersistentDomain(forName: suite) }
        let app = WindowSessions.shared
        defer { WindowSessions.shared = app }

        // The run before: two windows, each somewhere.
        let before = WindowSessions(defaults: defaults)
        WindowSessions.shared = before
        let firstWindow = before.adopt(), secondWindow = before.adopt()
        var one = firstWindow.session, two = secondWindow.session
        one.place = Self.workspace(Self.billing)
        two.place = Self.workspace(Self.shop)
        before.update(one)
        before.update(two)

        // ⌘Q, as the app delegate hears it, and the windows closing under it.
        let delegate = PushDelegate()
        #expect(delegate.applicationShouldTerminate(NSApplication.shared) == .terminateNow)
        before.closed(one.id)
        before.closed(two.id)
        delegate.applicationWillTerminate(Notification(name: NSApplication.willTerminateNotification))

        // The launch after: the first window takes a record and opens one
        // window more, which takes the other.
        let after = WindowSessions(defaults: defaults)
        WindowSessions.shared = after
        let opening = after.adopt()
        #expect(opening.open == 1, "the launch opens the second window")
        let opened = after.adopt()
        let records = [opening.session, opened.session]
        #expect(Set(records.compactMap(\.place)) == [Self.workspace(Self.billing), Self.workspace(Self.shop)])

        // Each window settles where it opens (`settleLaunch`) and its restore
        // task waits for the runner.
        let windows = records.map { record in
            let window = Window()
            if let kept = SelectionMemory.kept(destination: record.place?.encoded ?? "", legacy: "") {
                window.restoring = DestinationOpen(destination: kept, arrival: .restore, since: Date())
            }
            return window
        }
        #expect(windows.allSatisfy { $0.restoring != nil })
        let shown = windows.map(Self.show)
        defer { for window in shown { window.close() } }
        try await Self.settle(shown)

        // SwiftUI cancels the task and starts it again, as it did to every
        // window of the owner's relaunch, with the window's state intact.
        for window in windows { window.mounted = false }
        try await Self.settle(shown, for: .milliseconds(100))
        for window in windows { window.mounted = true }
        try await Self.settle(shown, for: .milliseconds(100))

        // The runner comes up with both workspaces.
        let up = World(
            seats: [
                World.Seat(
                    host: "", runnerId: "r1", ready: true,
                    workspaces: [World.Workspace(id: Self.billing), World.Workspace(id: Self.shop)], worktrees: [])
            ])
        for window in windows { window.world = up }
        let deadline = ContinuousClock.now + .seconds(5)
        while windows.contains(where: { $0.selection == nil }), ContinuousClock.now < deadline {
            try await Self.settle(shown, for: .milliseconds(50))
        }

        let expected = records.map { record -> ContentView.Selection? in
            record.place.flatMap { MacDestination.selection(for: $0, in: .empty) }
        }
        #expect(windows.map(\.selection) == expected, "a window opened on nothing")
        #expect(windows.allSatisfy { $0.restoring == nil }, "an open left waiting")
    }

    @Test("A cancelled run leaves its open for the next; any other end is done with it")
    func cancelledIsNotDone() {
        #expect(!WindowRestore.finished(.cancelled, cancelled: true))
        #expect(WindowRestore.finished(.cancelled, cancelled: false), "another open took its place")
        #expect(WindowRestore.finished(.opened, cancelled: true))
        #expect(WindowRestore.finished(.stayed(nil), cancelled: false))
    }
}
