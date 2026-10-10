import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the ruling row at rest and with its actions
/// shown (ov-418), and the task card's question (ov-431), at 320 and 900
/// points, in the four appearances. ("rest" has no actions at all, since an
/// offscreen window reports the pointer as over the row, and a hidden row's
/// text is laid out the same: the layout test pins that.) The production views, in a real titled
/// window off every screen, taken with `screencapture -l`. Sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct PlanUICaptures {
    static let decision =
        "Unread counts on the phones come from the daemon's own tally, never from the last list the phone happened to fetch, so a badge never lags the board."

    static func shoot(_ name: String, width: CGFloat, height: CGFloat, _ view: some View) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        for (variant, appearance) in RealWindowCaptures.variants {
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(
                rootView: view.padding(12).frame(width: width, height: height, alignment: .topLeading)
                    .background(Color(nsColor: .windowBackgroundColor)))
            window.appearance = NSAppearance(named: appearance)
            window.orderFrontRegardless()
            await NativeAgentTests.settle(window, 600)
            let image = try #require(RealWindowCaptures.windowImage(window), "screencapture -l isn't allowed here")
            try #require(image.representation(using: .png, properties: [:]))
                .write(to: URL(fileURLWithPath: out).appendingPathComponent("\(name)-\(variant).png"))
            window.close()
        }
    }

    @Test(arguments: [320.0, 900])
    func ruling(_ width: CGFloat) async throws {
        for shown in [false, true] {
            try await Self.shoot(
                "ruling-\(Int(width))-\(shown ? "hover" : "rest")", width: width, height: 220,
                PlanRulingRow(
                    ruling: PlanRuling(
                        id: "r", number: 34, decision: Self.decision, why: "It's the one attention color.",
                        reversal: "Two files change."),
                    copy: { _ in }
                )
                .environment(\.planRulingActions, PlanRulingActions(canKeep: shown, canAsk: shown, alwaysShown: shown)))
        }
    }

    /// Now with a train and its lanes named by what they do (ov-461, ov-462): long
    /// titles, a summary, the integrating agent's state and spend, the slug second.
    @Test(arguments: [320.0, 900])
    func now(_ width: CGFloat) async throws {
        let plan = try PlanModel.decode(PlanViewTests.fixture())
        var train = plan.trains[0]
        train.summary = "Mac interface polish; Agents tell the orchestrator when they are done; Phones show the plan"
        let lanes = [
            ("Agents tell the orchestrator when they are done", "agent-msg", LaneState.building),
            ("Mac interface polish", "mac-ux", LaneState.review),
            ("The start-line test is deterministic", "fix-ac84", LaneState.fixing),
        ].enumerated().map { index, row in
            var lane = plan.lanes[1]
            lane.id = "lane-\(index)"
            lane.name = row.1
            lane.title = row.0 == row.1 ? nil : row.0
            lane.state = row.2
            return lane
        }
        try await Self.shoot(
            "now-\(Int(width))", width: width, height: 460,
            VStack(alignment: .leading, spacing: 8) {
                PlanTrainRow(train: train, ci: plan.ci(of: train), now: plan.nowMs)
                ForEach(lanes) { lane in
                    PlanLaneRow(lane: lane, theme: nil, rank: nil, now: plan.nowMs, waitsOnOwner: false, selected: false, keyed: false, action: {})
                        .padding(.leading, 16)
                }
            })
    }

    @Test(arguments: [320.0, 900])
    func question(_ width: CGFloat) async throws {
        let options = TaskQuestionOptionsTests.options
        try await Self.shoot(
            "question-\(Int(width))", width: width, height: 360,
            QuestionAnswers(
                offer: TaskCard.Offer(
                    question: TaskQuestion(id: "q", body: "How should the ruling actions behave?", options: options),
                    options: options, typed: false),
                onAnswer: { _ in true }, draft: .none))
    }
}
