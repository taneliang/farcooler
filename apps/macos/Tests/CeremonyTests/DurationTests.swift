import Foundation
import Testing

@testable import Far_Cooler

/// The duration a row shows is a function of the time it is HANDED, not of the
/// time it reads.
///
/// That distinction is the whole of the fix, and it is invisible in a
/// screenshot: `Date()` read inside the property is an input SwiftUI cannot
/// observe, so nothing invalidated a working row and its duration froze until
/// an unrelated event forced a redraw — clicking a different pane, which is
/// how the complaint was phrased. Taking `now` as an argument is what lets a
/// `TimelineView` drive it once a second.
///
/// A test cannot see a redraw. It can see that the same terminal, asked twice,
/// answers differently — and a property reading its own clock would answer the
/// same both times when handed two different ones.
struct DurationTests {
    private func working(startedSecondsAgo: Double, at now: Date) -> Terminal {
        var t = Terminal(id: "1", short: "1", title: "claude", preset: "claude", state: "running", epoch: 0)
        t.activity = "working"
        t.turnStartedAt = (now.timeIntervalSince1970 - startedSecondsAgo) * 1000
        t.activitySince = t.turnStartedAt
        return t
    }

    /// A fixed instant, because the arithmetic here has to be exact.
    ///
    /// `working(startedSecondsAgo:)` sends the timestamp through milliseconds
    /// and `brief` brings it back — `((t - 30) * 1000) / 1000` — and with a
    /// `Date()` carrying a fractional second that round trip can land on
    /// `30 - epsilon`, which `Int(seconds)` truncates to 29. This test failed
    /// in CI exactly once that way. A whole second is exactly representable, so
    /// the round trip is lossless and the boundary is the boundary.
    ///
    /// The truncation itself is right and stays: elapsed time reads down, so a
    /// turn 29.9 seconds old says 29s. And nothing outside a test builds
    /// `turnStartedAt` this way — the daemon sends whole milliseconds.
    private static let fixedNow = Date(timeIntervalSince1970: 1_700_000_000)

    @Test func aWorkingRowsDurationAdvancesWithTheTimeItIsGiven() {
        let now = Self.fixedNow
        let terminal = working(startedSecondsAgo: 30, at: now)
        #expect(terminal.displayDuration(at: now) == "30s")
        #expect(terminal.displayDuration(at: now.addingTimeInterval(31)) == "1m")
        #expect(terminal.displayDuration(at: now.addingTimeInterval(3600)) == "1h")
    }

    /// Under five seconds there is nothing worth saying, so the label stands
    /// alone — and the row does not flicker a `0s` into existence the instant
    /// a turn begins.
    @Test func aTurnTooYoungToMeasureShowsNoDuration() {
        let now = Self.fixedNow
        #expect(working(startedSecondsAgo: 2, at: now).displayDuration(at: now) == nil)
    }
}
