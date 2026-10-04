import Foundation
import Testing

@testable import Far_Cooler

/// The watching claim is renewed while any window shows a pane, whichever
/// window called last (ov-133 review H1). Every window shares one client per
/// runner now, so a covered or idle second window saying it shows nothing
/// cancelled the renewal the first had armed, and the runner aged the claim
/// out after ten seconds: a banner for a pane in plain sight.
extension StartTaskTests {
    /// A clock held still, stepped by hand: each `tick` lets one pause end.
    @MainActor final class HeldClock {
        var now = Date(timeIntervalSince1970: 1_000_000)
        private var waiting: [CheckedContinuation<Void, Never>] = []
        /// A tick nobody was waiting for yet, kept for the next pause.
        private var owed = 0
        func pause() async {
            if owed > 0 {
                owed -= 1
                return
            }
            await withCheckedContinuation { waiting.append($0) }
        }
        func tick(_ seconds: TimeInterval) {
            now += seconds
            guard !waiting.isEmpty else {
                owed += 1
                return
            }
            let ready = waiting
            waiting = []
            ready.forEach { $0.resume() }
        }
    }

    @Test func oneWindowWatchingKeepsTheClaimAliveWhenAnotherShowsNothing() async {
        let runner = aFinishedAgent()
        let client = await client(runner)
        client.presence = Self.present()
        let clock = HeldClock()
        client.watchingPause = { await clock.pause() }
        client.watchingNow = { clock.now }
        let a = UUID()
        defer { Notifier.shared.closeWindow(a) }

        // Window A shows t-new; window B, covered, shows nothing, and calls last.
        Notifier.shared.setWatching(["t-new"], window: a)
        client.reportWatching(["t-new"])
        client.reportWatching([])
        await afterTheSentinel(client, runner)
        let before = runner.watchingCalls.count

        // Fifteen seconds, held: past the runner's ten, in renewals of four.
        for _ in 0..<4 {
            let sent = runner.watchingCalls.count
            clock.tick(4)
            // Until the renewal's call lands, or a second without one.
            for _ in 0..<100 where runner.watchingCalls.count == sent {
                try? await Task.sleep(for: .milliseconds(10))
            }
        }
        let renewals = runner.watchingCalls.dropFirst(before)
        #expect(renewals.count >= 3, "the claim was not renewed: \(runner.watchingCalls)")
        #expect(renewals.allSatisfy { $0.contains("t-new") }, "\(runner.watchingCalls)")
    }
}
