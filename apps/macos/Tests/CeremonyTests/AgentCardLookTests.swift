import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The agent chat's cards (ov-223): approval and failure are amber fills with
/// no outline, and the glass family over the transcript has one radius.
@MainActor
struct AgentCardLookTests {
    private func bitmap<V: View>(_ view: V, size: CGSize, _ name: NSAppearance.Name = .aqua) throws -> NSBitmapImageRep {
        let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height, alignment: .topLeading))
        host.appearance = NSAppearance(named: name)
        host.frame = CGRect(origin: .zero, size: size)
        host.layoutSubtreeIfNeeded()
        let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: rep)
        return rep
    }

    private func color(_ rep: NSBitmapImageRep, _ x: Int, _ y: Int) throws -> NSColor {
        try #require(rep.colorAt(x: x * rep.pixelsWide / Int(rep.size.width), y: y * rep.pixelsHigh / Int(rep.size.height)))
            .usingColorSpace(.sRGB)!
    }

    private let pending = {
        var transcript = Transcript()
        transcript.apply([
            Sequenced(
                seq: 1,
                event: .permission(id: "p", toolCall: "t", options: [PermissionOption(id: "a", name: "Allow", kind: "allow_once")]))
        ])
        return transcript.pendingPermission!
    }()

    @Test("An approval card's edge is its fill: no amber outline")
    func approvalHasNoOutline() throws {
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let rep = try bitmap(ApprovalCard(pending: pending, onChoose: { _ in }), size: CGSize(width: 320, height: 110), name)
            // The left edge at mid height, against just inside it.
            #expect(try color(rep, 0, 55) == color(rep, 3, 55), "an outline is drawn in \(name.rawValue)")
        }
    }

    @Test("A failure card's edge is its fill too")
    func failureHasNoOutline() throws {
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let rep = try bitmap(
                AgentFailureRow(failure: .notAuthenticated, agent: "Claude"), size: CGSize(width: 460, height: 150), name)
            // The row pads 16 around its card, so its edge is 16 in.
            #expect(try color(rep, 16, 60) == color(rep, 19, 60), "an outline is drawn in \(name.rawValue)")
        }
    }

    @Test("The plan, the queue and the composer share one concentric radius, never under six")
    func oneRadiusForTheFamily() {
        #expect(ChatFamily.radius == Radius.concentric(outer: Radius.medium, padding: 10))
        #expect(ChatFamily.radius >= Radius.small)
        // The composer was 20 and the queue's bubble 14.
        #expect(ChatFamily.radius < 14)
    }
}
