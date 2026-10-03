import Foundation
import Testing

@testable import AgentKit

/// `test/fixtures/glance-in-app.json`, which Android's `GlanceTest` reads too:
/// one terminal's activity, how long that state has lasted, and whether its
/// runner is answering, against the mark the app draws.
private struct GlanceInAppFixture: Decodable {
    struct Case: Decodable {
        struct Expect: Decodable {
            var attention: String
            var core: String?
            var link: String
        }
        var name: String
        var activity: String
        var stateAgeSeconds: Double?
        var answering: Bool
        var expect: Expect
    }
    var cases: [Case]

    static func load() throws -> GlanceInAppFixture {
        var root = URL(fileURLWithPath: #filePath)
        // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
        for _ in 0..<6 { root.deleteLastPathComponent() }
        let data = try Data(contentsOf: root.appendingPathComponent("test/fixtures/glance-in-app.json"))
        return try JSONDecoder().decode(GlanceInAppFixture.self, from: data)
    }
}

private func words(_ mark: GlanceMark) -> [String?] {
    let attention =
        switch mark.attention {
        case .needsYou: "needsYou"
        case .toReview: "toReview"
        case .quiet: "quiet"
        }
    let core: String? =
        switch mark.core {
        case .producing: "producing"
        case .atAPrompt: "atAPrompt"
        case nil: nil
        }
    let link =
        switch mark.link {
        case .live: "live"
        case .broken: "broken"
        }
    return [attention, core, link]
}

/// How long a state has lasted never dashes a ring; only a runner that isn't
/// answering does. Each case dates `activitySince` `stateAgeSeconds` before the
/// wall clock, so the rule that measured from the last state change (an hour,
/// against `activityChangedAt`) is red: "working for three hours on a live
/// link" came out dashed.
@Test("A long-lived state on a live link is not drawn stale")
func aLongLivedStateOnALiveLinkIsNotDrawnStale() throws {
    let fixture = try GlanceInAppFixture.load()
    #expect(fixture.cases.count >= 10)
    let now = Date().timeIntervalSince1970 * 1000
    for item in fixture.cases {
        let terminal = Terminal(
            id: "t", short: "t", title: "claude", preset: "claude", state: "running",
            activity: item.activity,
            activitySince: item.stateAgeSeconds.map { now - $0 * 1000 },
            epoch: 1)
        let mark = GlanceMark(terminal: terminal).said(answering: item.answering)
        #expect(
            words(mark) == [item.expect.attention, item.expect.core, item.expect.link],
            "\(item.name)")
    }
}
