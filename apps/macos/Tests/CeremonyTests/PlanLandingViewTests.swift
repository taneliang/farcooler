import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Older runners may still send numbered landing rows; the Plan does not draw them.
@MainActor
@Suite(.serialized)
struct PlanLandingViewTests {
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
