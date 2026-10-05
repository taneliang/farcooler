import AgentKit
import AppKit
import Foundation
import SwiftUI
import Testing
import os

@testable import Far_Cooler

/// A card on a quiet board turns stale on the minute: its metadata line
/// reads "No movement for 1d", with no data change (ov-92: the
/// mark is quiet text now, no orange sentence, icon or border).
///
/// The board redraws only when a task changes, and a card crosses a day of
/// silence at whatever minute it crosses it. So the line is drawn inside
/// `BoardTick`, and this draws the real `TaskListRow` in an unshown window,
/// moves the board's clock past the day, and reads the change back out of
/// the pixels. A line drawn from `Date()` outside a tick stays as it was.
///
/// Ink, not a question put to the view, because a view asked whether it is
/// stale will answer from its model while painting the last redraw: the
/// defect this guards IS a correct model drawn late.
@MainActor
struct BoardCardTickTests {
    /// The board's clock, held by the test and moved by it.
    final class Clock: Sendable {
        private let moment: OSAllocatedUnfairLock<Date>
        init(_ start: Date) { moment = OSAllocatedUnfairLock(initialState: start) }
        var now: Date { moment.withLock { $0 } }
        func move(by interval: TimeInterval) { moment.withLock { $0 += interval } }
        /// Twenty ticks a second, so the test waits a fraction of one rather
        /// than a minute.
        var board: BoardClock { BoardClock(interval: 0.05) { [self] in now } }
    }

    /// The card's pixels, to compare a redraw against.
    ///
    /// The process's appearance is Aqua for the draw and no longer: the pane
    /// chrome the border is stroked over resolves its system color against
    /// `NSApp.effectiveAppearance` (`blend` in Theme.swift), so on a Mac in
    /// Dark mode the card resolves against dark chrome and reads as too dark
    /// to count, whatever the host view says. It is set and put back within
    /// this synchronous call. The test used to hold it across the awaits
    /// below, five seconds in which every other main-actor test ran under a
    /// process-wide Aqua they never asked for (ov-252).
    private static func pixels(_ view: NSView) -> [UInt32] {
        let app = NSApplication.shared
        let appearance = app.appearance
        app.appearance = NSAppearance(named: .aqua)
        defer { app.appearance = appearance }
        let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds)!
        view.cacheDisplay(in: view.bounds, to: rep)
        var out: [UInt32] = []
        for y in 0..<rep.pixelsHigh {
            for x in 0..<rep.pixelsWide {
                var p = [Int](repeating: 0, count: 4)
                rep.getPixel(&p, atX: x, y: y)
                out.append(UInt32(p[0]) << 24 | UInt32(p[1]) << 16 | UInt32(p[2]) << 8 | UInt32(p[3]))
            }
        }
        return out
    }

    private static func differing(_ a: [UInt32], _ b: [UInt32]) -> Int {
        zip(a, b).filter { $0 != $1 }.count + abs(a.count - b.count)
    }

    @Test func aQuietCardTurnsStaleOnTheMinute() async throws {
        // Now, on the real clock, so a mark that reads `Date()` instead of the
        // board's clock sees the card as fresh — which is what a quiet board
        // does to it.
        let clock = Clock(Date())
        // In progress, and silent for a minute short of a day.
        let since = clock.now.addingTimeInterval(-TaskRow.staleAfter + 60)
        let row = TaskRow(
            id: "0198f2c0-0000-7000-8000-00000000c029", key: "-29", title: "A quiet card",
            status: .inProgress, statusSince: since, createdAt: since, updatedAt: since)
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let store = TaskBoardStore(
            client: client,
            workspace: .implicit(repository: "r"))
        let card = TaskListRow(
            row: row, prominent: false, store: store, live: [], presence: .unsaid,
            onGoTo: { _ in })
        // The card at a column's width and its own height, and the window
        // exactly that, so the regions `ink` reads are the card's and not
        // margin the window centers it in.
        let host = NSHostingView(
            rootView: card.frame(width: 260).fixedSize().environment(\.boardClock, clock.board))
        host.appearance = NSAppearance(named: .aqua)
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 260, height: 120), styleMask: [.borderless],
            backing: .buffered, defer: false)
        // Ours to let go of: an AppKit window releases itself on `close()`
        // by default, and ARC then releases it again.
        window.isReleasedWhenClosed = false
        window.contentView = host
        defer { window.close() }
        window.setContentSize(host.fittingSize)
        host.layoutSubtreeIfNeeded()

        let before = Self.pixels(host)
        let line = { TaskRowMeta.line(row, at: clock.now).lead }
        #expect(line()?.hasPrefix("Updated") == true || line()?.hasPrefix("Added") == true, "fresh: \(line() ?? "")")

        // Two minutes on: a day and a minute of silence. Nothing about the
        // card has changed but the time.
        clock.move(by: 120)
        #expect(line() == "No movement for 1d")
        var changed = 0
        let deadline = Date().addingTimeInterval(5)
        repeat {
            try await Task.sleep(for: .milliseconds(50))
            changed = Self.differing(before, Self.pixels(host))
        } while changed <= 20 && Date() < deadline
        #expect(changed > 20, "the stale line did not tick: \(changed) pixels changed")
    }
}
