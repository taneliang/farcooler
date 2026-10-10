import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Now draws trains as row groups on the Mac (ov-309): the fixture's integ-9,
/// red, heads mac-ux, which sits under it and indented; and a train whose CI
/// moves washes as a lane does. The words are AgentKit's (`PlanTrainsTests`).
@MainActor
@Suite(.serialized)
struct PlanTrainViewTests {
    static let trainID = "00000000-0000-0000-0000-000000006001"

    /// The fixture with integ-9 moved to `state`.
    static func plan(movingTrain state: String) throws -> Data {
        var object = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        var trains = try #require(object["trains"] as? [[String: Any]])
        trains[0]["state"] = state
        object["trains"] = trains
        return try JSONSerialization.data(withJSONObject: object)
    }

    @Test("A train heads its lanes in Now, above them and with them indented; its CI moving washes it")
    func aTrainHeadsItsLanes() async throws {
        let calls = PlanViewTests.Calls()
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        let seen = NavigatorFilterTests.Seen()
        let heard = PlanChangesTests.Heard()
        let host = NSHostingView(
            rootView: PlanChangesTests.Hosted(store: store, selection: PlanChangesTests.Selection(), seen: seen, heard: heard))
        host.frame = CGRect(x: 0, y: 0, width: 320, height: 900)
        func settle(_ frames: Int = 10) async {
            for _ in 0..<frames {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
        await settle()
        let train = try #require(seen.views["plan-train-integ-9"], "the train is drawn: \(seen.views.keys)")
        let lane = try #require(seen.views["plan-lane-mac-ux"])
        #expect(train.maxY <= lane.minY + 1, "the train heads its lane: \(train) \(lane)")
        // A frame this test sets, at any scale: the indent is the navigator's
        // mark column, many points, so a point either way doesn't decide it.
        #expect(lane.minX > train.minX + 4, "the lane sits under the train, indented: \(train) \(lane)")
        #expect(store.plan.plan.nowGroups.first?.train?.name == "integ-9")

        heard.events = []
        calls.plan = try Self.plan(movingTrain: "green")
        await store.plan.reload()
        await settle()
        #expect(heard.washed.contains(Self.trainID), "the train that moved washes: \(heard.washed)")
    }

    /// The fixture with a live lane named like integ-9, flagged as the runner flags it (ov-461).
    static func plan(addingShadowLane: Bool) throws -> Data {
        var object = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        var lanes = try #require(object["lanes"] as? [[String: Any]])
        var shadow = lanes[1]
        shadow["id"] = "00000000-0000-0000-0000-000000002099"
        shadow["name"] = "integ-9"
        shadow["title"] = ""
        shadow["train"] = NSNull()
        lanes.append(shadow)
        object["lanes"] = lanes
        object["lane_is_train"] = [["lane": shadow["id"] as Any, "name": "integ-9"]]
        return try JSONSerialization.data(withJSONObject: object)
    }

    @Test("A train reads Train 9 with what it carries and its slug second; a lane named like it is not drawn again")
    func aTrainReadsByTitleAndIsDrawnOnce() async throws {
        let calls = PlanViewTests.Calls()
        calls.plan = try Self.plan(addingShadowLane: true)
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        let seen = NavigatorFilterTests.Seen()
        let host = NSHostingView(
            rootView: PlanChangesTests.Hosted(store: store, selection: PlanChangesTests.Selection(), seen: seen, heard: PlanChangesTests.Heard()))
        host.frame = CGRect(x: 0, y: 0, width: 320, height: 900)
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        #expect(seen.views["plan-train-integ-9"] != nil && seen.views["plan-train-integ-9-carries"] != nil, "\(seen.views.keys)")
        #expect(seen.views["plan-lane-mac-ux"] != nil)
        #expect(seen.views["plan-lane-integ-9"] == nil, "the train stands for the lane: \(seen.views.keys)")
        let group = try #require(store.plan.plan.nowGroups.first)
        #expect((group.train?.heading, group.train?.slug, group.lanes.map(\.heading)) == ("Train 9", "integ-9", ["Mac interface polish"]))
    }
}
