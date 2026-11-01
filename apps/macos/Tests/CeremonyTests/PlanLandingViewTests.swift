import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Now draws landings as row groups on the Mac (ov-309): the fixture's Landing
/// 9, red, carries ov-4, which sits inside its card and indented; and a
/// landing whose CI moves washes as a ticket does. The words are AgentKit's
/// (`PlanLandingTests`).
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

    @Test("A landing is a card in Landing with its tickets inside it; its CI moving washes it")
    func aLandingHeadsItsTickets() async throws {
        let calls = PlanViewTests.Calls()
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        let (host, seen, heard) = Self.hosted(store)
        await Self.settle(host)
        let landing = try #require(seen.views["plan-landing-9"], "the landing is drawn: \(seen.views.keys)")
        let ticket = try #require(seen.views["plan-landing-ticket-ov-4"], "its ticket is inside its card: \(seen.views.keys.sorted())")
        #expect(landing.minY <= ticket.minY && ticket.maxY <= landing.maxY + 1, "the ticket is inside the landing's card: \(landing) \(ticket)")
        // A frame this test sets, at any scale: the indent is the navigator's
        // mark column, many points, so a point either way doesn't decide it.
        #expect(ticket.minX > landing.minX + 4, "the ticket sits inside the card, inset: \(landing) \(ticket)")
        #expect(store.plan.plan.nowGroups.first?.landing?.number == 9)

        heard.events = []
        calls.plan = try Self.plan(movingLanding: "green")
        await store.plan.reload()
        await Self.settle(host)
        #expect(heard.washed.contains(Self.landingID), "the landing that moved washes: \(heard.washed)")
    }

    @Test("A landing reads Landing 9 with what it carries; its ticket is drawn once, inside the card")
    func aLandingReadsByLabelAndIsDrawnOnce() async throws {
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults())
        await store.plan.reload()
        let (host, seen, _) = Self.hosted(store)
        await Self.settle(host)
        #expect(seen.views["plan-landing-9-label"] != nil && seen.views["plan-landing-9-carries"] != nil, "\(seen.views.keys)")
        // Its title, "Landing 9", says no more than its label: it is not said again.
        #expect(seen.views["plan-landing-9-subtitle"] == nil, "\(seen.views.keys.sorted())")
        #expect(seen.views["plan-ticket-ov-4"] == nil, "the landing stands for the ticket: \(seen.views.keys.sorted())")
        let group = try #require(store.plan.plan.nowGroups.first)
        #expect((group.landing?.label, group.lines.map(\.heading)) == ("Landing 9", ["Title of ov-4"]))
    }
}
