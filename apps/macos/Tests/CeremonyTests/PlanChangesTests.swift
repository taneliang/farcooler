import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The plan's lists move as the task list does when the plan changes
/// (ov-298, `listChanges`): a ticket or theme that arrives or changes washes,
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

    static let ticketID = "00000000-0000-0000-0000-000000001002"  // ov-2, waiting and ranked first

    /// The fixture with ov-2's reason for being next changed to `state`.
    static func plan(movingTicketReason state: String) throws -> Data {
        var object = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        var lines = try #require(object["lines"] as? [[String: Any]])
        let at = try #require(lines.firstIndex { $0["task"] as? String == ticketID })
        lines[at]["reason"] = state
        object["lines"] = lines
        return try JSONSerialization.data(withJSONObject: object)
    }

    @Test("A ticket whose reason moves washes and changes on the spring; a selection moving does neither")
    func changedTicketFlashesSelectionDoesNot() async throws {
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
        #expect(seen.views["plan-ticket-ov-2"] != nil, "the ticket is drawn: \(seen.views.keys)")
        #expect(heard.washed.isEmpty, "nothing washes on the first draw")

        // The selection moves onto the ticket: no wash, and nothing animated.
        heard.events = []
        selection.page = .ticket(Self.ticketID)
        await settle()
        #expect(heard.washed.isEmpty, "a selection isn't a change")
        #expect(!heard.animated.isEmpty, "the rows were drawn again for the selection")
        #expect(!heard.animated.contains(true), "the selection ran animated: \(heard.animated)")

        // The runner changes ov-2's reason for being next: it washes, on the
        // spring, and nothing else does.
        heard.events = []
        calls.plan = try Self.plan(movingTicketReason: "Now the Mac slot is free")
        await store.plan.reload()
        await settle()
        #expect(Set(heard.washed) == [Self.ticketID], "only the ticket that changed washes: \(heard.washed)")
        #expect(heard.animated.contains(true), "the change ran on the spring: \(heard.animated)")
    }
}
