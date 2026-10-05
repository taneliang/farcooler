import Foundation
import Testing

@testable import AgentKit

// The plan fits every surface it's drawn on (ov-310 review H3), measured as
// rendered pixels: each style at the smallest surface it goes on, at the
// largest text size the reader can pick, with the longest names the runner
// sends and every caveat. `PlanGlanceView` sizes its own caption from the
// reader's text size, so a Mac renders the size an iPhone would.
#if os(macOS)
    import AppKit
    import SwiftUI

    @MainActor
    struct PlanGlanceFitTests {
        /// The widest a runner sends: a 48-byte board, two 60-character lanes
        /// and a 60-character next up, with a count.
        static let widest = PlanGlance(
            workspace: String(repeating: "W", count: 48), needsYou: 12,
            now: [
                .init(name: String(repeating: "a", count: 60), state: .review),
                .init(name: String(repeating: "b", count: 60), state: .landing),
            ],
            next: String(repeating: "n", count: 60), runner: "Studio", heardAgo: 0)

        static let caveats: [PlanCaveat?] = [nil, PlanCaveat(age: 3 * 3600), PlanCaveat(age: 3 * 3600, cantReach: "Studio")]

        /// The view's height at `width`, as it lays itself out.
        func height<V: View>(_ view: V, width: CGFloat) throws -> CGFloat {
            let renderer = ImageRenderer(
                content: view.frame(width: width).fixedSize(horizontal: false, vertical: true)
                    .environment(\.dynamicTypeSize, .accessibility5))
            renderer.scale = 1
            return CGFloat(try #require(renderer.cgImage).height)
        }

        @Test("An accessory's four lines fit the smallest lock screen and Smart Stack slots")
        func accessories() throws {
            // The smallest lock screen rectangular (157 × 66) and a 40 mm
            // watch's (162 × 69).
            for caveat in Self.caveats {
                let drawn = try height(PlanGlanceView(Self.widest, style: .lines, caveat: caveat), width: 157)
                #expect(drawn <= 66, "\(drawn) pt with \(String(describing: caveat))")
            }
        }

        @Test("A widget's rows fit the smallest small widget inside its margins")
        func widgetRows() throws {
            // An iPhone SE's small widget, 141 points, less 16 a side.
            for caveat in Self.caveats {
                let drawn = try height(PlanGlanceView(Self.widest, style: .rows, caveat: caveat), width: 109)
                #expect(drawn <= 109, "\(drawn) pt with \(String(describing: caveat))")
            }
        }

        @Test("The Live Activity's rows card keeps its 160 points with the plan on it")
        func card() throws {
            let update = try #require(try Contracts.object("live-activity/running/plan.json")["aps"] as? [String: Any])
            var card = try JSONDecoder().decode(
                AgentCardState.self, from: JSONSerialization.data(withJSONObject: try #require(update["content-state"])))
            // Two rows, the most the card draws.
            var second = try #require(card.rows.first)
            second.terminal = "term-two"
            card.rows.append(second)
            let layout = try #require(
                AgentCardLayout(state: card, now: Date(timeIntervalSince1970: 1_791_019_830), stale: false))
            for caveat in Self.caveats {
                for stale in [false, true] {
                    let plan = PlanGlanceView(Self.widest, style: .card, caveat: caveat, stale: stale)
                    // The narrowest lock screen card: 375 less 16 a side.
                    let drawn = try height(GlanceCardView(layout: layout, plan: plan), width: 343)
                    #expect(drawn <= 160, "\(drawn) pt with \(String(describing: caveat)), stale \(stale)")
                }
            }
        }

        @Test("A lane's name is cut with an ellipsis before its state is")
        func names() {
            #expect(PlanGlanceView.shown(String(repeating: "a", count: 60)) == String(repeating: "a", count: 19) + "…")
            #expect(PlanGlanceView.shown("mac-ux") == "mac-ux")
            #expect(PlanGlanceView.captionPoints(.accessibility5) == 18)
        }
    }
#endif
