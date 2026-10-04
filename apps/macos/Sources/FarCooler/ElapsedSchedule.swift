import SwiftUI

/// Wake-ups for a duration label, at the moments its text can change (ov-229).
///
/// The label reads `Working 42s`, then `Working 12m`, then `Working 3h`
/// (`Terminal.brief`): nothing until five seconds, seconds until a minute,
/// minutes until an hour, then hours. A clock ticking every second redrew
/// such a row sixty times for each change it could show. This one wakes at
/// the next boundary only, counted from when the clock started.
struct ElapsedSchedule: TimelineSchedule {
    /// When the row's clock started. Nil wakes on the wall clock's minutes.
    let since: Date?

    func entries(from start: Date, mode: TimelineScheduleMode) -> Entries {
        Entries(upcoming: start, since: since)
    }

    struct Entries: Sequence, IteratorProtocol {
        var upcoming: Date
        let since: Date?

        mutating func next() -> Date? {
            let current = upcoming
            upcoming = ElapsedSchedule.wake(after: current, since: since)
            return current
        }
    }

    /// The first moment after `now` at which the label can read differently.
    static func wake(after now: Date, since: Date?) -> Date {
        guard let since else {
            return Date(timeIntervalSince1970: ((now.timeIntervalSince1970 / 60).rounded(.down) + 1) * 60)
        }
        let elapsed = now.timeIntervalSince(since)
        if elapsed < 5 { return since.addingTimeInterval(5) }
        let step: TimeInterval =
            switch elapsed {
            case ..<60: 1
            case ..<3600: 60
            default: 3600
            }
        return since.addingTimeInterval(((elapsed / step).rounded(.down) + 1) * step)
    }
}

/// Every `every` seconds while `running`, and only the first moment otherwise.
///
/// For a view that needs the time only in one state, such as an orchestrator
/// still starting. `.periodic` would wake it every few seconds in every state
/// and rebuild everything inside it (ov-229).
struct WhileSchedule: TimelineSchedule {
    let every: TimeInterval
    let running: Bool

    func entries(from start: Date, mode: TimelineScheduleMode) -> AnyIterator<Date> {
        var upcoming: Date? = start
        return AnyIterator {
            defer { upcoming = running ? upcoming?.addingTimeInterval(every) : nil }
            return upcoming
        }
    }
}
