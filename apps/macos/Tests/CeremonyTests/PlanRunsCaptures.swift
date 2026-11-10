import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the Plan's overview with the engine's run on three tickets (ov-1040), and the same plan
/// without them, in the production `PlanOverviewView` at a narrow width with long titles, light and dark. The plans are
/// `test/fixtures/plan-runs.json` and `plan-runs-before.json`.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct PlanRunsCaptures {
    static func plan(_ name: String) throws -> Data {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let file = try JSONSerialization.jsonObject(with: Data(contentsOf: root.appendingPathComponent("test/fixtures/\(name).json")))
        return try JSONSerialization.data(withJSONObject: try #require((file as? [String: Any])?["plan"]))
    }

    @Test("The Building tickets carry their run's step and gate", arguments: ["plan-runs", "plan-runs-before"], ["light", "dark"])
    func narrow(name: String, scheme: String) async throws {
        let out = URL(fileURLWithPath: try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]))
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let calls = PlanViewTests.Calls()
        calls.plan = try Self.plan(name)
        let store = try await PlanViewTests.store(plan: true, defaults: PlanViewTests.defaults(), calls: calls)
        await store.plan.reload()
        let view = PlanOverviewView(
            plan: store.plan, statuses: store.board.statuses, selected: nil, needsYou: 1, onOpen: { _ in }, defaults: PlanViewTests.defaults()
        )
        .frame(width: 360)
        .background(Color(nsColor: .windowBackgroundColor))
        .preferredColorScheme(scheme == "dark" ? .dark : .light)
        let host = NSHostingView(rootView: view)
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 360, height: 4200), styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: scheme == "dark" ? .darkAqua : .aqua)
        window.contentView = host
        TestWindow.show(window)
        defer { window.close() }
        try await Capture.keep(
            host, to: out.appendingPathComponent("\(name)-mac-360-\(scheme).png"), expecting: "the plan's Building section"
        ) {
            host.layoutSubtreeIfNeeded()
            return host.fittingSize.height > 600
        }
    }
}
