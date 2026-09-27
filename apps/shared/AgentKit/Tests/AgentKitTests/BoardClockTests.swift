import AppKit
import Foundation
import SwiftUI
import Testing
import os

@testable import AgentKit

/// A clock a test moves, read by `BoardClock.now`.
final class MovableClock: Sendable {
    private let moment: OSAllocatedUnfairLock<Date>
    init(_ start: Date) { moment = OSAllocatedUnfairLock(initialState: start) }
    var now: Date { moment.withLock { $0 } }
    func move(by interval: TimeInterval) { moment.withLock { $0 += interval } }
    /// Ticking twenty times a second, so a test waits a fraction of a second
    /// rather than a minute.
    var board: BoardClock { BoardClock(interval: 0.05) { [self] in now } }
}

/// The schedule lands on the wall clock's minutes, after drawing at once.
@Test func theBoardTickDrawsAtOnceThenOnEachMinute() {
    let start = Date(timeIntervalSince1970: 1_757_170_830.25)  // 30.25 s into a minute
    let entries = Array(
        BoardTickSchedule(interval: 60).entries(from: start, mode: .normal).prefix(3))
    #expect(entries == [
        start,
        Date(timeIntervalSince1970: 1_757_170_860),
        Date(timeIntervalSince1970: 1_757_170_920),
    ])
    // A start on the boundary is drawn once there, and next a minute on.
    let onIt = Date(timeIntervalSince1970: 1_757_170_860)
    #expect(
        Array(BoardTickSchedule(interval: 60).entries(from: onIt, mode: .normal).prefix(2))
            == [onIt, Date(timeIntervalSince1970: 1_757_170_920)])
    #expect(BoardClock.wall.interval == 60)
}

/// What `BoardTick` last handed its content, written from the view.
private final class Seen: Sendable {
    private let moment = OSAllocatedUnfairLock<Date?>(initialState: nil)
    var last: Date? { moment.withLock { $0 } }
    func saw(_ date: Date) { moment.withLock { $0 = date } }
}

private struct Probe: View {
    let seen: Seen
    var body: some View {
        BoardTick { now in
            let _ = seen.saw(now)
            Text("tick")
        }
    }
}

/// **The tick redraws without a data change, and hands in the clock's time.**
///
/// The real view in a real (unshown) window: nothing about `Probe` changes
/// after it is first drawn, so the only thing that can draw it again is the
/// `TimelineView`. Take that out of `BoardTick` and the content is drawn once,
/// at the first moment, and this fails.
@MainActor
@Test func aBoardTickRedrawsOnTheClockAlone() async throws {
    let clock = MovableClock(Date(timeIntervalSince1970: 1_757_170_830))
    let seen = Seen()
    let host = NSHostingView(rootView: Probe(seen: seen).environment(\.boardClock, clock.board))
    let window = NSWindow(
        contentRect: NSRect(x: 0, y: 0, width: 120, height: 40), styleMask: [.borderless],
        backing: .buffered, defer: false)
    // Ours to let go of: an AppKit window releases itself on `close()`
    // by default, and ARC then releases it again.
    window.isReleasedWhenClosed = false
    window.contentView = host
    defer { window.close() }
    host.layoutSubtreeIfNeeded()
    #expect(seen.last == clock.now, "the first draw reads the clock")

    clock.move(by: 86_400)
    let deadline = Date().addingTimeInterval(5)
    while seen.last != clock.now, Date() < deadline {
        try await Task.sleep(for: .milliseconds(20))
    }
    #expect(seen.last == clock.now, "no tick redrew the content with the moved clock")
}
