import Foundation
import SwiftUI

/// The clock a board's cards read, and how often they read it again.
///
/// A card's time-dependent look — "Hasn’t moved in a day", "Updated 2h ago",
/// the stale border and its clock icon — is computed against a moment, and the
/// board redraws only when a task changes. On a quiet board nothing else would
/// ever hand a card a new moment, so a card filed at six would still read
/// "Added just now" at eleven, and one that crossed a day of silence would say
/// so in words inside a card drawn as though it had not.
///
/// An environment value rather than `Date()` written into each view, so a
/// test can hand the real card a clock it moves, on a tick fast enough to
/// watch, and see the card redraw without a data change. `wall` is the only
/// clock the apps install.
public struct BoardClock: Sendable {
    /// What time it is. Read inside each tick, never taken from the tick: the
    /// schedule's entry for a minute is that minute's start, up to sixty
    /// seconds in the past, and a sentence built on it reads a whole unit
    /// short ("Updated 9m ago" for ten minutes).
    public var now: @Sendable () -> Date
    /// How often a card reads `now` again, in seconds, on boundaries aligned
    /// to multiples of it: sixty is every wall-clock minute.
    public var interval: TimeInterval

    public init(interval: TimeInterval, now: @escaping @Sendable () -> Date) {
        self.interval = interval
        self.now = now
    }

    /// The wall clock, read again on every minute.
    public static let wall = BoardClock(interval: 60) { Date() }

    /// When a card redraws: the moment it is first drawn, then every
    /// `interval` boundary after.
    public var schedule: BoardTickSchedule { BoardTickSchedule(interval: interval) }
}

/// `BoardClock`'s schedule: `start` itself, then each multiple of `interval`
/// after it, measured from the Unix epoch — so a sixty-second interval lands
/// on the wall clock's minutes, as `.everyMinute` does. Ours rather than
/// `.everyMinute` so a test can ask for a faster one without the view knowing.
public struct BoardTickSchedule: TimelineSchedule, Sendable {
    public let interval: TimeInterval

    public init(interval: TimeInterval) {
        self.interval = interval
    }

    public func entries(from start: Date, mode: TimelineScheduleMode) -> Entries {
        Entries(upcoming: start, interval: interval)
    }

    public struct Entries: Sequence, IteratorProtocol {
        var upcoming: Date
        let interval: TimeInterval

        public mutating func next() -> Date? {
            let current = upcoming
            let boundary = (current.timeIntervalSince1970 / interval).rounded(.down) + 1
            upcoming = Date(timeIntervalSince1970: boundary * interval)
            return current
        }
    }
}

extension EnvironmentValues {
    /// The clock the board's cards read; see `BoardClock`.
    @Entry public var boardClock: BoardClock = .wall
}

/// Draws `content` against the board's clock, and again on each of its ticks.
///
/// Wrap only what depends on the time — a sentence, an icon, a border — so a
/// tick redraws that piece and not the card or the board around it. Every
/// time-dependent piece of a card goes through one of these; a piece that
/// reads the time outside one is drawn at the last data change and stays
/// there on a quiet board.
public struct BoardTick<Content: View>: View {
    @Environment(\.boardClock) private var clock
    private let content: (Date) -> Content

    public init(@ViewBuilder content: @escaping (Date) -> Content) {
        self.content = content
    }

    public var body: some View {
        // The tick decides WHEN to redraw, and the clock what time it is.
        TimelineView(clock.schedule) { _ in
            content(clock.now())
        }
    }
}
