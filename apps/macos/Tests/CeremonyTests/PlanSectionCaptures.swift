import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the Plan's home at 680, 960 and 1,400 points
/// in the four appearances, over scenarios with long real titles (ov-503, phase
/// 5). The production `PlanHome`, over a `TaskBoardStore` that reads a plan
/// through the client's own command runner, in a real titled window off every
/// screen, taken with `screencapture -l`. Sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct PlanSectionCaptures {
    static func shoot(
        _ name: String, kind: PlanScenario.Kind, width: CGFloat, height: CGFloat, expecting: String,
        until reached: @MainActor (TaskBoardStore) -> Bool
    ) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let calls = PlanViewTests.Calls()
        calls.plan = try PlanScenario.data(kind)
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        for (variant, appearance) in RealWindowCaptures.variants {
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(
                rootView: PlanHome(board: store, needsYou: PlanNeedsYou(), onOpen: { _ in })
                    .frame(width: width, height: height, alignment: .topLeading)
                    .background(WorkspaceStyle.canvas))
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            try await Capture.wait(expecting: expecting, until: { reached(store) })
            await NativeAgentTests.settle(window, 900)
            try await Capture.keep(to: URL(fileURLWithPath: out).appendingPathComponent("\(name)-\(Int(width))-\(variant).png"), expecting: expecting, until: { reached(store) }) { RealWindowCaptures.windowImage(window)?.representation(using: .png, properties: [:]) }
            window.close()
        }
    }

    @Test(arguments: [680.0, 960.0, 1400.0])
    func busy(_ width: CGFloat) async throws {
        try await Self.shoot(
            "plan-busy", kind: .busy, width: width, height: width < 900 ? 2900 : 1900, expecting: "the busy plan read",
            until: { $0.plan.planned })
    }

    @Test(arguments: [680.0, 1400.0])
    func emptyNext(_ width: CGFloat) async throws {
        try await Self.shoot(
            "plan-empty-next", kind: .emptyNext, width: width, height: width < 900 ? 2300 : 1500,
            expecting: "the plan with nothing next, read", until: { $0.plan.planned })
    }

    @Test(arguments: [680.0, 1400.0])
    func quiet(_ width: CGFloat) async throws {
        try await Self.shoot(
            "plan-quiet", kind: .quiet, width: width, height: 900, expecting: "the quiet plan read", until: { $0.plan.planned })
    }
}
