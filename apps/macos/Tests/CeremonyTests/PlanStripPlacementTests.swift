import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Train 1004r, P4: the strip lay over the orchestrator's last rows, its
/// input box and status line. It is a row of its own, now the cards' at the
/// top of the column (ov-641), and the terminal starts under it: never over
/// the terminal's rows or its input.
@MainActor
@Suite(.serialized)
struct PlanStripPlacementTests {
    final class Seen { var frames: [String: CGRect] = [:] }

    @Test("The cards sit above the terminal, and are inside the window, with the canvas or without")
    func terminalStartsBelowTheCards() async throws {
      for width in [CGFloat(1000), 1700] {
        let seen = Seen()
        let view = WorkspaceView(
            opened: nil as String?, hasConversation: true, cell: WorkspaceColumns.defaultCell, focused: false,
            navigatorWidth: .constant(320), conversation: { Color.clear }, navigator: { Color.clear },
            breadcrumb: { _ in Color.clear }, detail: { _, _ in Color.clear }, split: true,
            cards: { _ in AnyView(Color.clear.frame(width: 300, height: 30).probed("strip-stub")) })
            .frame(width: width, height: 700)
            .environment(\.gridProbing, true)
            .overlayPreferenceValue(ProbedViewsKey.self) { probed in
                GeometryReader { proxy in
                    let _ = seen.frames = Dictionary(
                        probed.map { ($0.id, proxy[$0.bounds]) }, uniquingKeysWith: { first, _ in first })
                    Color.clear
                }
            }
        let host = NSHostingView(rootView: view)
        host.frame = CGRect(x: 0, y: 0, width: width, height: 700)
        for _ in 0..<5 {
            host.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(20))
        }
        let body = try #require(seen.frames["workspace-conversation-body"], "drawn: \(seen.frames.keys)")
        let strip = try #require(seen.frames["strip-stub"])
        #expect(strip.maxY <= body.minY + 0.5, "width \(width): the cards cover the terminal's first \(strip.maxY - body.minY) pt")
        #expect(strip.minY >= -0.5 && body.maxY <= 700.5)
      }
    }
}
