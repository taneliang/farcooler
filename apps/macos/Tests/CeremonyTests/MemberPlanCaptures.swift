import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the Plan's home, light, on the owner's Mac
/// and on a member's (ov-637). A member sees the plan the owner does, the
/// themes' asks of the owner included, and is offered no Keep or Keep All,
/// which the runner refuses them. The busy scenario's long real titles, with
/// two rulings standing.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct MemberPlanCaptures {
    /// The busy plan with the fixture's rulings back, and a second standing one
    /// so the owner is offered Keep All.
    static func plan() throws -> Data {
        var object = try #require(try JSONSerialization.jsonObject(with: PlanScenario.data(.busy)) as? [String: Any])
        let fixture = try #require(try JSONSerialization.jsonObject(with: PlanViewTests.fixture()) as? [String: Any])
        var rulings = try #require(fixture["rulings"] as? [[String: Any]])
        var more = rulings[0]
        more["id"] = "00000000-0000-0000-0000-000000005003"
        more["number"] = 3
        more["short"] = "R-3"
        more["decision"] = "Invoices export through pdfkit, the library the receipts export already uses."
        rulings.insert(more, at: 0)
        object["rulings"] = rulings
        return try JSONSerialization.data(withJSONObject: object)
    }

    static func reached(_ store: TaskBoardStore) -> Bool {
        PlanScenario.shown(.busy, in: store.plan.plan) && store.plan.plan.rulings.count == 3
    }

    @Test(arguments: [("owner", ""), ("member", "bob")])
    func home(viewer: String, you: String) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        let calls = PlanViewTests.Calls()
        calls.plan = try Self.plan()
        let target = "capture-plan-\(viewer)-\(UUID().uuidString)"
        defer { UserDefaults.standard.removeObject(forKey: "talk.you.\(target)") }
        let store = try await PlanViewTests.store(
            plan: true, defaults: PlanViewTests.defaults(), calls: calls, target: target, you: you)
        await store.client.talk.load(store.client, force: true)
        await store.plan.reload()
        #expect(store.plan.canMarkRulings == you.isEmpty)
        let actions = PlanRulingActions(canKeep: store.plan.canMarkRulings, canAsk: true, alwaysShown: true)
        let (width, height): (CGFloat, CGFloat) = (960, 2050)
        let window = NSWindow(
            contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = NSHostingView(
            rootView: PlanHome(board: store, needsYou: PlanNeedsYou(), onOpen: { _ in })
                .environment(\.planRulingActions, actions)
                .frame(width: width, height: height, alignment: .topLeading)
                .background(WorkspaceStyle.canvas))
        window.appearance = NSAppearance(named: .aqua)
        TestWindow.show(window)
        let expecting = "the busy plan with its rulings, read"
        try await Capture.wait(expecting: expecting, until: { Self.reached(store) })
        try await Capture.keep(
            to: URL(fileURLWithPath: out).appendingPathComponent("member-plan-\(viewer)-light.png"), expecting: expecting,
            until: { Self.reached(store) }
        ) { window.contentView?.lookBitmap(scale: 2)?.representation(using: .png, properties: [:]) }
        window.close()
    }
}
