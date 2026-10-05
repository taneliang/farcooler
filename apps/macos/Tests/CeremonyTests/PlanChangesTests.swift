import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The plan's lists move as the task list does when the plan changes
/// (ov-298, `listChanges`): a lane or theme that arrives or changes washes,
/// and the change runs on the shared spring. A selection moving does neither
/// (ov-293: drawn on the next frame, no transition).
@MainActor
@Suite(.serialized)
struct PlanChangesTests {
    /// The page the overview draws selected, as a test moves it.
    final class Selection: ObservableObject {
        @Published var page: PlanPage?
    }

    /// What the rows told: each transaction, and each wash.
    final class Heard {
        var events: [ListChangeEvent] = []
        var animated: [Bool] {
            events.compactMap { if case .drawn(_, let animated) = $0 { animated } else { nil } }
        }
        var washed: [String] { events.compactMap { if case .washed(let id) = $0 { id } else { nil } } }
    }

    struct Hosted: View {
        let store: TaskBoardStore
        @ObservedObject var selection: Selection
        let seen: NavigatorFilterTests.Seen
        let heard: Heard

        var body: some View {
            PlanOverviewView(
                plan: store.plan, statuses: store.board.statuses, selected: selection.page, keyed: true, onOpen: { _ in })
                .frame(width: 320, height: 900, alignment: .topLeading)
                .environment(\.gridProbing, true)
                .environment(\.listChangeProbe) { heard.events.append($0) }
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { proxy in
                        let _ = seen.views = Dictionary(
                            probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                        Color.clear
                    }
                }
        }
    }

    static let laneID = "00000000-0000-0000-0000-000000002002"  // mac-ux, in review

    /// The fixture with mac-ux moved to `state`.
    static func plan(movingMacUX state: String) throws -> Data {
        var object = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        var lanes = try #require(object["lanes"] as? [[String: Any]])
        let at = try #require(lanes.firstIndex { $0["id"] as? String == laneID })
        lanes[at]["state"] = state
        object["lanes"] = lanes
        return try JSONSerialization.data(withJSONObject: object)
    }

    @Test("A lane whose state moves washes and changes on the spring; a selection moving does neither")
    func changedLaneFlashesSelectionDoesNot() async throws {
        let calls = PlanViewTests.Calls()
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        let selection = Selection()
        let seen = NavigatorFilterTests.Seen()
        let heard = Heard()
        let host = NSHostingView(rootView: Hosted(store: store, selection: selection, seen: seen, heard: heard))
        host.frame = CGRect(x: 0, y: 0, width: 320, height: 900)
        func settle(_ frames: Int = 10) async {
            for _ in 0..<frames {
                host.layoutSubtreeIfNeeded()
                try? await Task.sleep(for: .milliseconds(20))
            }
        }
        await settle()
        #expect(seen.views["plan-lane-mac-ux"] != nil, "the lane is drawn: \(seen.views.keys)")
        #expect(heard.washed.isEmpty, "nothing washes on the first draw")

        // The selection moves onto the lane: no wash, and nothing animated.
        heard.events = []
        selection.page = .lane(Self.laneID)
        await settle()
        #expect(heard.washed.isEmpty, "a selection isn't a change")
        #expect(!heard.animated.isEmpty, "the rows were drawn again for the selection")
        #expect(!heard.animated.contains(true), "the selection ran animated: \(heard.animated)")

        // The runner says mac-ux went back to fixing: it washes, on the spring,
        // and so does the one theme it moves, whose track line changes with
        // it (ov-331: "mac-ux is in review" is now "mac-ux is fixing"); nothing
        // else does.
        heard.events = []
        calls.plan = try Self.plan(movingMacUX: "fixing")
        await store.plan.reload()
        await settle()
        #expect(
            Set(heard.washed) == [Self.laneID, "00000000-0000-0000-0000-000000003001"] && heard.washed.count == 2,
            "only the lane that changed, and the theme it moves, wash: \(heard.washed)")
        #expect(heard.animated.contains(true), "the change ran on the spring: \(heard.animated)")
    }
}
