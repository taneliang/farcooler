import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A section is as tall as its rows on the first layout pass (ov-298). The
/// owner's live build drew a gap under the last theme card that closed a
/// moment later: a height taken from a measurement that arrived a beat late
/// (a geometry reading, a lazy grid's estimate), then corrected.
@MainActor
@Suite(.serialized)
struct FirstLayoutHeightTests {
    /// What was drawn where, read on the pass that drew it.
    final class Seen {
        var frames: [String: CGRect] = [:] {
            didSet { for (id, frame) in frames { history[id, default: []].append(frame) } }
        }
        /// Every frame each probe was drawn at, in order, from the first.
        var history: [String: [CGRect]] = [:]
    }

    static func host(_ view: some View, seen: Seen, size: CGSize) -> NSHostingView<AnyView> {
        let host = NSHostingView(
            rootView: AnyView(
                view
                    .frame(width: size.width, height: size.height, alignment: .topLeading)
                    .environment(\.gridProbing, true)
                    .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                        GeometryReader { proxy in
                            let _ = seen.frames = Dictionary(
                                probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                            Color.clear
                        }
                    }))
        host.frame = CGRect(origin: .zero, size: size)
        return host
    }

    /// `count` rows, each `row` tall.
    static func rows(_ count: Int, row: CGFloat) -> some View {
        VStack(spacing: 0) { ForEach(0..<count, id: \.self) { _ in Color.clear.frame(height: row) } }
    }

    @Test("A navigator pane is as tall as its rows on the first pass, with no correction after")
    func paneOnFirstPass() async throws {
        let seen = Seen()
        var kept = ""
        let panes = [
            NavigatorSplitPane(id: "themes", expanded: true, header: AnyView(Text("Themes")), content: AnyView(Self.rows(3, row: 28))),
            NavigatorSplitPane(id: "tasks", fills: true, expanded: true, header: AnyView(Text("Tasks")), content: AnyView(Self.rows(7, row: 28))),
        ]
        let host = Self.host(
            NavigatorSplitView(panes: panes, kept: Binding(get: { kept }, set: { kept = $0 })), seen: seen,
            size: CGSize(width: 300, height: 900))
        // One pass: nothing has had a runloop turn to measure itself.
        host.layoutSubtreeIfNeeded()
        let first = try #require(seen.frames["navigator-pane-themes"], "drawn: \(seen.frames.keys)")
        // 84 pt of rows plus the rule's room under them (`paneInset`).
        let rows = 3 * 28 + NavigatorSplit.paneInset
        #expect(abs(first.height - rows) < 0.5, "first drawn \(first.height) pt for \(rows) pt of rows")
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(20))
        }
        let drawn = seen.history["navigator-pane-themes"] ?? []
        #expect(drawn.allSatisfy { abs($0.height - rows) < 0.5 }, "drawn at \(drawn.map(\.height)) before \(rows)")
    }

    @Test("The overview's theme cards are their real height on the first pass: what's under them doesn't move")
    func themesOnFirstPass() async throws {
        let calls = PlanViewTests.Calls()
        // The fixture's landed lane is "today" only in some time zones: it
        // landed at 07:10 UTC and the plan's clock reads 08:00 UTC, so on CI
        // (UTC) a Landed Today section sits under the themes and its header
        // is counted as a gap, while on a Pacific Mac it falls on the day
        // before and there's none. Move it three days back, so the overview
        // ends at the themes in every zone.
        var plan = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        var lanes = try #require(plan["lanes"] as? [[String: Any]])
        for index in lanes.indices where lanes[index]["state"] as? String == "landed" {
            lanes[index]["state_since"] = 1_800_000_000_000 - 3 * 24 * 3_600_000
        }
        plan["lanes"] = lanes
        // And no Cost section (ov-307) under them either: this is about the
        // theme cards' height, so the overview must end at them.
        plan["cost"] = NSNull()
        calls.plan = try JSONSerialization.data(withJSONObject: plan)
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        let seen = Seen()
        let host = Self.host(
            // In a scroll view, as the canvas and the peek draw it.
            ScrollView {
                VStack(spacing: 0) {
                    PlanOverviewView(plan: store.plan, statuses: store.board.statuses, selected: nil, onOpen: { _ in })
                    Color.clear.frame(height: 1).probed("after")
                }
            },
            seen: seen, size: CGSize(width: 320, height: 1400))
        host.layoutSubtreeIfNeeded()
        let first = try #require(seen.frames["after"], "drawn: \(seen.frames.keys)")
        let card = try #require(seen.frames["plan-theme-entry-Visual language"])
        for _ in 0..<10 {
            host.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(20))
        }
        let drawn = (seen.history["after"] ?? []).map(\.minY)
        #expect(drawn.allSatisfy { abs($0 - first.minY) < 0.5 }, "what follows the cards was drawn at \(drawn)")
        // Under the last card is the overview's bottom padding and nothing
        // else, so the gap is `Spacing.section`: a card measured short and
        // corrected later leaves a different one. The 1 pt is for CI's 1x
        // layout, which snaps text heights to whole points (at most half a
        // point a line); the bug being caught is a card's worth of points.
        let gap = first.minY - card.maxY
        #expect(abs(gap - Spacing.section) < 1, "a gap under the last card: \(gap) pt, expected \(Spacing.section)")
    }
}
