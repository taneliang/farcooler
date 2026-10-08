import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A ruling row keeps its buttons their natural size (ov-405): a long
/// decision wraps in the space the buttons leave, and never crushes Keep,
/// Reverse, Discuss or the copy icon into columns a character wide. The
/// natural size is measured in the same environment, from the same row drawn
/// wide enough that nothing competes, so no font metric is hard-coded.
@MainActor
@Suite(.serialized)
struct PlanRulingRowLayoutTests {
    static let long =
        "Unread counts on the phones come from the daemon's own tally, never from the last list the phone happened to fetch, so a badge never lags the board."
    static let ids = ["keep", "reverse", "discuss", "copy"]

    struct Hosted: View {
        let width: CGFloat
        let seen: NavigatorFilterTests.Seen
        let decision: String
        var alwaysShown = true

        var body: some View {
            PlanRulingRow(
                ruling: PlanRuling(
                    id: "r", number: 34, decision: decision, why: "It's the one attention color.",
                    reversal: "Two files change."),
                copy: { _ in }
            )
            .environment(\.planRulingActions, PlanRulingActions(canKeep: true, canAsk: true, alwaysShown: alwaysShown))
            .padding(.horizontal, 12)
            .frame(width: width, alignment: .topLeading)
            .frame(maxHeight: .infinity, alignment: .topLeading)
            .environment(\.gridProbing, true)
            .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                GeometryReader { proxy in
                    let _ = seen.views = Dictionary(
                        probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                    Color.clear
                }
            }
        }
    }

    /// The row's probed frames at `width` points, with `decision`.
    static func frames(width: CGFloat, decision: String = long, alwaysShown: Bool = true) async -> [String: CGRect] {
        let seen = NavigatorFilterTests.Seen()
        let host = NSHostingView(rootView: Hosted(width: width, seen: seen, decision: decision, alwaysShown: alwaysShown))
        let window = NavigatorFilterTests.KeyWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: width, height: 400), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = host
        window.makeKeyAndOrderFront(nil)
        defer { window.close() }
        for _ in 0..<15 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return seen.views
    }

    @Test("At every narrow width the buttons keep the size they have in a wide pane, and the decision wraps instead")
    func buttonsKeepTheirSize() async throws {
        let wide = await Self.frames(width: 1400)
        for id in Self.ids { #expect(wide["plan-ruling-R-34-\(id)"] != nil, "\(id): \(wide.keys.sorted())") }
        let oneLine = try #require(wide["plan-ruling-R-34-decision"]).height
        for width in [240.0, 300, 360, 480] {
            let narrow = await Self.frames(width: width)
            for id in Self.ids {
                let key = "plan-ruling-R-34-\(id)"
                let natural = try #require(wide[key])
                let got = try #require(narrow[key], "\(key) at \(width)")
                // A crushed button is a character wide, so any shortfall of a
                // point is the bug; a point of slack covers pixel snapping at 1x.
                #expect(got.width >= natural.width - 1, "\(key) at \(width): \(got.width) of \(natural.width)")
                #expect(got.height <= natural.height + 1, "\(key) at \(width) wrapped: \(got.height) of \(natural.height)")
            }
            let decision = try #require(narrow["plan-ruling-R-34-decision"])
            #expect(decision.height > oneLine * 1.5, "the decision wraps at \(width)")
            #expect(decision.maxX <= width, "the decision stays inside the row at \(width)")
        }
    }

    @Test("In a short row the actions sit on their own line under the decision; in a wide one they share its first line")
    func shortRowsMoveTheActionsDown() async throws {
        let wide = await Self.frames(width: 1400)
        let wideDecision = try #require(wide["plan-ruling-R-34-decision"])
        for id in ["keep", "reverse", "discuss"] {
            let button = try #require(wide["plan-ruling-R-34-\(id)"])
            // The same line: the top of a button is within a line of the decision's top.
            #expect(button.minY < wideDecision.minY + wideDecision.height, "\(id) shares the line at 1400")
        }
        for width in [240.0, 300, 360] {
            let narrow = await Self.frames(width: width)
            let decision = try #require(narrow["plan-ruling-R-34-decision"])
            let copy = try #require(narrow["plan-ruling-R-34-copy"])
            for id in ["keep", "reverse", "discuss"] {
                let button = try #require(narrow["plan-ruling-R-34-\(id)"])
                #expect(button.minY >= decision.maxY - 1, "\(id) under the decision at \(width): \(button.minY) vs \(decision.maxY)")
                #expect(button.maxX <= width, "\(id) inside the row at \(width)")
            }
            // With the actions gone from beside it, the decision has the row but the copy icon.
            #expect(decision.maxX <= copy.minX + 1, "the decision stops before the copy icon at \(width)")
            #expect(decision.width >= (width - 24) * 0.6, "the decision gets most of the row at \(width): \(decision.width)")
        }
    }

    @Test("Hidden actions hold no width: the decision runs right up to the copy icon, and hovering moves nothing")
    func hiddenActionsReserveNothing() async throws {
        // Wide enough for the actions to sit over the decision, narrow enough
        // for it to wrap, so its frame is the room it was given.
        for width in [500.0, 700] {
            let shown = await Self.frames(width: width, alwaysShown: true)
            let hidden = await Self.frames(width: width, alwaysShown: false)
            let decision = try #require(shown["plan-ruling-R-34-decision"])
            let keep = try #require(shown["plan-ruling-R-34-keep"])
            let copy = try #require(shown["plan-ruling-R-34-copy"])
            let actions = try #require(shown["plan-ruling-R-34-discuss"]).maxX - keep.minX
            #expect(actions > 60, "the actions have a width to reserve: \(actions)")
            // A wrapped line ends short of the edge by up to a word, never by
            // the actions' width, so a reservation shows as a shortfall of it.
            #expect(
                decision.maxX >= copy.minX - Spacing.group - actions / 2,
                "the decision stops short of the copy icon at \(width): \(decision.maxX) vs \(copy.minX)")
            #expect(decision.maxX > keep.minX + 1, "the decision runs under the actions at \(width)")
            #expect(keep.maxX <= copy.minX, "the actions sit left of the copy icon at \(width)")
            // Showing the actions changes no line of the decision.
            let still = try #require(hidden["plan-ruling-R-34-decision"])
            #expect(abs(still.width - decision.width) < 1 && abs(still.height - decision.height) < 1, "hover reflows at \(width)")
        }
    }
}
