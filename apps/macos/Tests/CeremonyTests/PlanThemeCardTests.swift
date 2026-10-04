import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A theme's card in the Plan view (ov-273): its outcome wraps to three lines,
/// the owner's ruling, and no further.
@MainActor
@Suite(.serialized)
struct PlanThemeCardTests {
    /// The height each probed view drew at, in a card `width` wide.
    static func heights(_ view: some View, width: CGFloat) async -> [String: CGFloat] {
        let seen = NavigatorFilterTests.Seen()
        let host = NSHostingView(
            rootView: view
                .frame(width: width)
                .fixedSize(horizontal: false, vertical: true)
                .environment(\.gridProbing, true)
                .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                    GeometryReader { proxy in
                        let _ = seen.views = Dictionary(
                            probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                        Color.clear
                    }
                })
        host.frame = CGRect(x: 0, y: 0, width: width, height: 800)
        for _ in 0..<5 {
            host.layoutSubtreeIfNeeded()
            try? await Task.sleep(for: .milliseconds(20))
        }
        return seen.views.mapValues(\.height)
    }

    /// One line of the outcome's type, measured here, in this environment.
    static func line(width: CGFloat) async throws -> CGFloat {
        let one = await heights(
            Text("Outcome").font(PlanThemeCard.outcomeFont).lineLimit(1).probed("line"), width: width)
        return try #require(one["line"])
    }

    static func theme(outcome: String) throws -> PlanTheme {
        let model = try PlanModel.decode(try PlanViewTests.fixture())
        var theme = try #require(model.themes.first)
        theme.outcome = outcome
        return theme
    }

    @Test("A long outcome shows three lines in the card at its narrowest")
    func threeLines() async throws {
        // At the overview's narrowest card (its adaptive minimum, 220 pt),
        // this runs to six lines or more.
        let long = String(repeating: "Every board shows its plan beside its tasks, legible on its own. ", count: 4)
        let card = PlanThemeCard(theme: try Self.theme(outcome: long), selected: false, keyed: false) {}
        let drawn = await Self.heights(card, width: 220)
        let outcome = try #require(drawn["plan-theme-outcome"], "the outcome was drawn: \(drawn.keys)")
        let lines = outcome / (try await Self.line(width: 220))
        #expect(lines > 2.5 && lines < 3.5, "the outcome drew \(lines) lines")
    }

    @Test("A short outcome takes the lines it needs")
    func shortOutcome() async throws {
        let card = PlanThemeCard(theme: try Self.theme(outcome: "Plan beside tasks."), selected: false, keyed: false) {}
        let drawn = await Self.heights(card, width: 220)
        let outcome = try #require(drawn["plan-theme-outcome"])
        let lines = outcome / (try await Self.line(width: 220))
        #expect(lines > 0.5 && lines < 1.5, "the outcome drew \(lines) lines")
    }
}
