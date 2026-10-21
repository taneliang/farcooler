import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the Plan home and the sidebar's Lanes list
/// with a train in every state of the ladder, at a narrow and a wide width, in
/// the four appearances (ov-522). The production views in a real titled window
/// off every screen, taken with `screencapture -l`. Sends no input.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct PlanLandingCaptures {
    static func shoot(
        _ name: String, width: CGFloat, height: CGFloat, expecting: String, until reached: @MainActor () -> Bool, _ view: some View
    ) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        for (variant, appearance) in RealWindowCaptures.variants {
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(
                rootView: view.frame(width: width, height: height, alignment: .topLeading).background(WorkspaceStyle.canvas))
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            try await Capture.wait(expecting: expecting, until: reached)
            await NativeAgentTests.settle(window, 900)
            try await Capture.keep(to: URL(fileURLWithPath: out).appendingPathComponent("\(name)-\(Int(width))-\(variant).png"), expecting: expecting, until: reached) { RealWindowCaptures.windowImage(window)?.representation(using: .png, properties: [:]) }
            window.close()
        }
    }

    static func store() async throws -> TaskBoardStore {
        let calls = PlanViewTests.Calls()
        calls.plan = try PlanScenario.data(.ladder)
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        return store
    }

    @Test(arguments: [480.0, 1200.0])
    func plan(_ width: CGFloat) async throws {
        let store = try await Self.store()
        try await Self.shoot(
            "landing-plan", width: width, height: 1900, expecting: "the ladder plan read", until: { PlanScenario.shown(.ladder, in: store.plan.plan) },
            PlanHome(board: store, needsYou: PlanNeedsYou(), onOpen: { _ in }))
    }

    @Test(arguments: [240.0, 380.0])
    func sidebar(_ width: CGFloat) async throws {
        let store = try await Self.store()
        let input = OneTreeInput(tasks: [], plan: store.plan.plan)
        let sidebar = OneTreeSidebar(
            tree: { _ in SidebarOutline.build(input) }, selected: nil, hint: nil, key: "landing|\(UUID().uuidString)", onChoose: { _ in })
        try await Self.shoot(
            "landing-sidebar", width: width, height: 1000, expecting: "the ladder plan read, in the sidebar",
            until: { PlanScenario.shown(.ladder, in: store.plan.plan) }, OneTreeNavigator(sidebar: sidebar, filterText: "", keyed: false))
    }
}
