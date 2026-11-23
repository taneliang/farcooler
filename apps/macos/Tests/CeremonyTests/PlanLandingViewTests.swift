import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Older runners may still send numbered landing rows; the Plan does not draw them.
@MainActor
@Suite(.serialized)
struct PlanLandingViewTests {
    static let landingID = "00000000-0000-0000-0000-000000006001"

    /// The fixture with Landing 9 moved to `state`.
    static func plan(movingLanding state: String) throws -> Data {
        var object = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        var landings = try #require(object["landings"] as? [[String: Any]])
        let at = try #require(landings.firstIndex { $0["id"] as? String == landingID })
        landings[at]["state"] = state
        // The runner derives the phase with the state; the card reads the phase.
        landings[at]["phase"] = state == "green" ? "ready_to_land" : landings[at]["phase"]
        object["landings"] = landings
        return try JSONSerialization.data(withJSONObject: object)
    }

    static func hosted(_ store: TaskBoardStore, heard: PlanChangesTests.Heard = PlanChangesTests.Heard())
        -> (host: NSHostingView<PlanChangesTests.Hosted>, seen: NavigatorFilterTests.Seen, heard: PlanChangesTests.Heard)
    {
        let seen = NavigatorFilterTests.Seen()
        let host = NSHostingView(
            rootView: PlanChangesTests.Hosted(store: store, selection: PlanChangesTests.Selection(), seen: seen, heard: heard))
        host.frame = CGRect(x: 0, y: 0, width: 320, height: 900)
        return (host, seen, heard)
    }

    static func settle(_ host: NSView, _ frames: Int = 10) async {
        for _ in 0..<frames {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
    }

    @Test("Numbered landings from an older runner do not render")
    func oldLandingIsHidden() async throws {
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults())
        await store.plan.reload()
        let (host, seen, _) = Self.hosted(store)
        await Self.settle(host)
        #expect(seen.views["plan-landing-9"] == nil)
        #expect(seen.views["plan-landing-ticket-ov-4"] == nil)
    }
}
