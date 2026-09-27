import AgentKit
import AppKit
import Foundation
import SwiftUI
import Testing
import os

@testable import Far_Cooler

/// A card on a quiet board turns stale on the minute: the orange sentence, the
/// orange border and the clock icon, together, with no data change.
///
/// The board redraws only when a task changes, and a card crosses a day of
/// silence at whatever minute it crosses it. So each of the card's three stale
/// marks is drawn inside `BoardTick`, and this draws the real `TaskCardRow` in
/// an unshown window, moves the board's clock past the day, and reads the
/// three marks back out of the pixels. A mark drawn from `Date()` outside a
/// tick — as the border and the icon were until ov-29 — stays grey here and
/// fails its own line, while the other two pass.
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

    /// Orange pixels in three places on the card: a band down its left edge
    /// (the border), its top-right corner (the icon), and the rest of its
    /// inside (the sentence — nothing else on this card is orange). The two
    /// inner regions keep six points off every edge, clear of the border's
    /// rounded corners, whose ink would otherwise count for the other marks.
    struct Ink: CustomStringConvertible {
        var border = 0, icon = 0, sentence = 0
        var description: String { "border \(border), icon \(icon), sentence \(sentence)" }
    }

    private static func ink(_ view: NSView) -> Ink {
        let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds)!
        view.cacheDisplay(in: view.bounds, to: rep)
        let scale = CGFloat(rep.pixelsWide) / view.bounds.width
        let (w, h) = (CGFloat(rep.pixelsWide), CGFloat(rep.pixelsHigh))
        let pt = { (value: CGFloat) in Int((value * scale).rounded()) }
        var ink = Ink()
        for y in 0..<rep.pixelsHigh {
            for x in 0..<rep.pixelsWide {
                guard let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB),
                    c.alphaComponent > 0.3, c.redComponent > 0.7,
                    c.redComponent - c.blueComponent > 0.3
                else { continue }
                let (fx, fy) = (CGFloat(x), CGFloat(y))
                if x < pt(1.5), fy > h * 0.3, fy < h * 0.7 {
                    ink.border += 1
                } else if fx > w - CGFloat(pt(32)), x < rep.pixelsWide - pt(6), y > pt(6),
                    y < pt(28)
                {
                    ink.icon += 1
                } else if x > pt(6), fx < w - CGFloat(pt(32)), y > pt(6),
                    y < rep.pixelsHigh - pt(6)
                {
                    ink.sentence += 1
                }
            }
        }
        return ink
    }

    @Test func aQuietCardTurnsStaleOnTheMinuteBorderIconAndSentence() async throws {
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
            repository: Repository(
                id: "r", short: "r", displayName: "r", remote: "", repositoryRootId: ""))
        let card = TaskCardRow(
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

        let before = Self.ink(host)
        #expect(before.border == 0 && before.icon == 0 && before.sentence == 0, "fresh: \(before)")

        // Two minutes on: a day and a minute of silence. Nothing about the
        // card has changed but the time.
        clock.move(by: 120)
        var after = Ink()
        let deadline = Date().addingTimeInterval(5)
        repeat {
            try await Task.sleep(for: .milliseconds(50))
            after = Self.ink(host)
        } while (after.border == 0 || after.icon == 0 || after.sentence == 0) && Date() < deadline

        #expect(after.sentence > 20, "the stale sentence did not tick: \(after)")
        #expect(after.border > 20, "the orange border did not tick: \(after)")
        #expect(after.icon > 20, "the clock icon did not tick: \(after)")
    }
}
