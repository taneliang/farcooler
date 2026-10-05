import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Train 1004r, P4: the strip lay over the orchestrator's last rows, its
/// input box and status line. Folded, it's a row of its own under the
/// terminal, which ends above it.
@MainActor
@Suite(.serialized)
struct PlanStripPlacementTests {
    final class Seen { var frames: [String: CGRect] = [:] }

    @Test("Folded, the terminal ends above the strip, and the strip is inside the window")
    func terminalEndsAboveTheStrip() async throws {
        let seen = Seen()
        let view = WorkspaceView(
            opened: nil as String?, hasConversation: true, cell: WorkspaceColumns.defaultCell, focused: false,
            navigatorWidth: .constant(320), conversation: { Color.clear }, navigator: { Color.clear },
            breadcrumb: { _ in Color.clear }, detail: { _, _ in Color.clear }, split: true,
            strip: { AnyView(Color.clear.frame(width: 300, height: 30).probed("strip-stub")) })
            .frame(width: 1000, height: 700)
            .environment(\.gridProbing, true)
            .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                GeometryReader { proxy in
                    let _ = seen.frames = Dictionary(
                        probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                    Color.clear
                }
            }
        let host = NSHostingView(rootView: view)
        host.frame = CGRect(x: 0, y: 0, width: 1000, height: 700)
        for _ in 0..<5 {
            host.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(20))
        }
        let body = try #require(seen.frames["workspace-conversation-body"], "drawn: \(seen.frames.keys)")
        let strip = try #require(seen.frames["strip-stub"])
        #expect(body.maxY <= strip.minY + 0.5, "the strip covers the terminal's last \(body.maxY - strip.minY) pt")
        #expect(strip.maxY <= 700.5)
    }
}
