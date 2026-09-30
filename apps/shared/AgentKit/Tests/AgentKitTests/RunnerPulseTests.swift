import Foundation
import Testing

@testable import AgentKit

/// Telling a live runner from a stale one while the app is suspended (ov-53).
///
/// The relay reports how long ago each beating runner was heard; these are
/// the phone's half: when that's quiet, and what the widget says about it.
struct RunnerPulseTests {
    /// Two missed beats and five minutes' slack: fifteen minutes at the
    /// shipped five-minute beat. Just inside is live; just past is quiet.
    ///
    /// Mutation: `quietAfter` returning `beatEvery` alone. Red.
    @Test func aRunnerIsQuietPastTwoMissedBeatsAndSlack() {
        #expect(RunnerPulse.quietAfter(beatEvery: 300) == 15 * 60)
        #expect(!RunnerPulse(label: "Studio", heardAgo: 14 * 60_000, beatEvery: 300).isQuiet)
        #expect(RunnerPulse(label: "Studio", heardAgo: 16 * 60_000, beatEvery: 300).isQuiet)
        // Judged by its own promise, not the shipped one.
        #expect(!RunnerPulse(label: "Slow", heardAgo: 16 * 60_000, beatEvery: 600).isQuiet)
    }

    /// Only the quiet ones are named, once each, in the relay's order.
    @Test func onlyQuietRunnersAreNamedOnce() {
        let pulses = [
            RunnerPulse(label: "This Mac", heardAgo: 3_600_000, beatEvery: 300),
            RunnerPulse(label: "Studio", heardAgo: 1_000, beatEvery: 300),
            RunnerPulse(label: "This Mac", heardAgo: 7_200_000, beatEvery: 300),
            RunnerPulse(label: "Attic", heardAgo: 3_600_000, beatEvery: 300),
        ]
        #expect(RunnerPulse.quiet(pulses) == ["This Mac", "Attic"])
    }

    /// The widget asks to be looked at again only while some runner beats;
    /// otherwise it keeps `.never` and the budget.
    ///
    /// Mutation: `nextLook` answering a date for an empty list. Red.
    @Test func theWidgetLooksAgainOnlyWhileARunnerBeats() {
        let now = Date(timeIntervalSince1970: 1_000)
        #expect(RunnerPulse.nextLook(after: now, pulses: nil) == nil)
        #expect(RunnerPulse.nextLook(after: now, pulses: []) == nil)
        #expect(
            RunnerPulse.nextLook(
                after: now, pulses: [RunnerPulse(label: "Studio", heardAgo: 0, beatEvery: 300)])
                == now.addingTimeInterval(30 * 60))
    }

    /// The relay's answer decodes; anything else is nil, never a throw.
    @Test func theRelaysAnswerDecodes() {
        let json = #"{"runners":[{"label":"Studio","heardAgo":1200,"beatEvery":300}]}"#
        #expect(
            RunnerPulse.decode(Data(json.utf8))
                == [RunnerPulse(label: "Studio", heardAgo: 1200, beatEvery: 300)])
        #expect(RunnerPulse.decode(Data(#"{"error":"unauthorized"}"#.utf8)) == nil)
    }

    /// A quiet runner gets ov-50's words, and outranks "from notifications".
    /// Nothing quiet is exactly today's hedge.
    ///
    /// Mutation: `hedge(quiet:)` ignoring `quiet`. Red.
    @Test func aQuietRunnerIsLostTouch() {
        let partial = FleetSnapshot(
            agents: [], capturedAt: Date(timeIntervalSince1970: 1), complete: false)
        #expect(partial.hedge(quiet: ["Studio"]) == .lostTouch(["Studio"]))
        #expect(partial.hedge(quiet: ["Studio"])?.footer == "lost touch with Studio")
        #expect(partial.hedge(quiet: []) == .fromNotifications)
        #expect(partial.hedge(quiet: []) == partial.hedge)

        let whole = FleetSnapshot(
            agents: [], capturedAt: Date(timeIntervalSince1970: 1), complete: true)
        #expect(whole.hedge(quiet: []) == nil)
        #expect(whole.hedge(quiet: ["Studio"]) == .lostTouch(["Studio"]))
    }

    /// A runner the app lost touch with AND the relay calls quiet is named
    /// once, the app's word first.
    @Test func aRunnerLostAndQuietIsNamedOnce() {
        let lost = FleetSnapshot(
            agents: [], capturedAt: Date(timeIntervalSince1970: 1), complete: false,
            lostRunners: ["Orchard"])
        #expect(lost.hedge(quiet: ["Studio", "Orchard"]) == .lostTouch(["Orchard", "Studio"]))
    }

    /// The credential round-trips through its file, and clearing removes it.
    @Test func theCredentialComesBackAndCanBeCleared() throws {
        let dir = URL(fileURLWithPath: NSTemporaryDirectory())
            .appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }

        #expect(PulseStore.read(fromContainer: dir) == nil)
        let credential = PulseCredential(relay: "https://relay.test", token: "abc")
        try PulseStore.write(credential, toContainer: dir)
        #expect(PulseStore.read(fromContainer: dir) == credential)
        PulseStore.clear(inContainer: dir)
        #expect(PulseStore.read(fromContainer: dir) == nil)
    }
}
