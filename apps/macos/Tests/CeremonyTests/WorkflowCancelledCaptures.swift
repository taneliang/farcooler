import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): a cancelled run with a long reason (ov-1053), in the production
/// `WorkflowRunsView` at its narrowest width (280 pt, its own minimum), light and dark. The card is the shared core's
/// own "cancelled" golden case, so the words are the ones every app shows. It also fails if the reason is cut off:
/// the view must be as tall as its text needs.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct WorkflowCancelledCaptures {
    static func cancelledCard() throws -> String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        let all = try JSONSerialization.jsonObject(
            with: Data(contentsOf: root.appendingPathComponent("crates/client/src/testdata/workflow-presentation.json"))) as! [[String: Any]]
        let fixture = try #require(all.first { $0["name"] as? String == "cancelled" })
        let card = (fixture["response"] as! [String: Any])["card"]!
        return String(decoding: try JSONSerialization.data(withJSONObject: card), as: UTF8.self)
    }

    @Test("A cancelled run says why, at the narrowest width", arguments: ["light", "dark"])
    func narrow(scheme: String) async throws {
        let out = URL(fileURLWithPath: try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"]))
        try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)
        let json = try Self.cancelledCard()
        let port = WorkflowPort(id: "capture", read: { _, _ in json }, dispatch: { _, _, _, _ in "" })
        UserDefaults.standard.set("item", forKey: "workflow.run.runner.workspace")
        let view = WorkflowRunsView(host: "runner", workspace: "workspace")
            .environment(\.workflowPort, port)
            .preferredColorScheme(scheme == "dark" ? .dark : .light)
        let host = NSHostingView(rootView: view)
        let window = NSWindow(
            contentRect: NSRect(x: -4000, y: -4000, width: 280, height: 1200),
            styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.appearance = NSAppearance(named: scheme == "dark" ? .darkAqua : .aqua)
        window.contentView = host
        TestWindow.show(window)
        defer { window.close(); UserDefaults.standard.removeObject(forKey: "workflow.run.runner.workspace") }
        // The loaded card is far taller than the empty sheet, so a tall layout is the state; and the window is taller
        // than the card needs, so the whole reason is on screen.
        try await Capture.keep(
            host, to: out.appendingPathComponent("workflow-cancelled-mac-280-\(scheme).png"),
            expecting: "the cancelled run's card with its reason"
        ) {
            host.layoutSubtreeIfNeeded()
            return host.fittingSize.height > 250
        }
        #expect(host.fittingSize.height < 1200, "the card is taller than the window, so the reason would be cut off")
    }
}
