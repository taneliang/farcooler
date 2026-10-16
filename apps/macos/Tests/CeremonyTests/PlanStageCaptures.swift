import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): a pull request's stage on the Plan home's
/// lane rows, the lane page and the card page's "In lane" line, light and dark
/// at 1x and 2x, with a long real reviewer login at a narrow and a wide width
/// (ov-316). The production views over a `TaskBoardStore` that reads the
/// runner's own bytes (`test/fixtures/plan-stages.json`) through the client's
/// command runner, in a real titled window off every screen. Sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct PlanStageCaptures {
    static func shoot<V: View>(_ name: String, width: CGFloat, height: CGFloat, _ view: V) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        for scale in [1, 2] {
            for (variant, appearance) in RealWindowCaptures.variants.prefix(2) {
                let window = NSWindow(
                    contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
                    backing: .buffered, defer: false)
                window.isReleasedWhenClosed = false
                window.contentView = NSHostingView(
                    rootView: view.frame(width: width, height: height, alignment: .topLeading)
                        .background(WorkspaceStyle.canvas))
                window.appearance = NSAppearance(named: appearance)
                TestWindow.show(window)
                await NativeAgentTests.settle(window, 900)
                let drawn = try #require(window.contentView?.lookBitmap(scale: scale))
                try #require(drawn.representation(using: .png, properties: [:]))
                    .write(to: URL(fileURLWithPath: out).appendingPathComponent("\(name)-\(Int(width))-\(scale)x-\(variant).png"))
                window.close()
            }
        }
    }

    static func store() async throws -> TaskBoardStore {
        let store = try await PlanStageViewTests.store()
        await store.plan.reload()
        return store
    }

    @Test(arguments: [440.0, 1000.0])
    func home(_ width: CGFloat) async throws {
        let store = try await Self.store()
        try await Self.shoot(
            "plan-stage-home", width: width, height: width < 700 ? 2200 : 1500,
            PlanHome(board: store, needsYou: PlanNeedsYou(), onOpen: { _ in }))
    }

    @Test(arguments: [(4, 520.0), (5, 380.0), (6, 520.0), (14, 520.0)])
    func lanePage(_ pick: (Int, CGFloat)) async throws {
        let store = try await Self.store()
        let plan = store.plan.plan
        try await Self.shoot(
            "plan-stage-lane-\(pick.0)", width: pick.1, height: 360,
            PlanLanePage(
                lane: plan.lanes[pick.0 - 1], plan: plan, record: nil,
                context: PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })))
    }

    @Test(arguments: [(4, 600.0), (5, 600.0), (5, 260.0), (6, 600.0)])
    func taskLine(_ pick: (Int, CGFloat)) async throws {
        let store = try await Self.store()
        try await Self.shoot(
            "plan-stage-card-\(pick.0)", width: pick.1, height: 110,
            PlanTaskLineView(plan: store.plan, task: PlanStageViewTests.task(pick.0)) { _ in })
    }
}
