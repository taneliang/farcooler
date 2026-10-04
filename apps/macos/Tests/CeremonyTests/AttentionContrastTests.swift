import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Attention cards under Increase Contrast (review 1004i M1): an approval, a
/// failure and a tool call waiting on you are amber washes with no outline,
/// and with Increase Contrast on, `Fill.inset` doubles under them and a 0.10
/// wash is lost. There each gets a 1 pt amber outline and a stronger fill.
///
/// Contrast is set through `\._colorSchemeContrast`, the writable twin of
/// `colorSchemeContrast` that previews use: a high-contrast NSAppearance
/// doesn't reach SwiftUI's value (checked below), and the public key is
/// read-only.
@MainActor
struct AttentionContrastTests {
    static let appearances: [NSAppearance.Name] = [.aqua, .darkAqua]

    private func bitmap<V: View>(
        _ view: V, size: CGSize, _ name: NSAppearance.Name, contrast: ColorSchemeContrast = .increased, scale: Int
    ) throws -> NSBitmapImageRep {
        let host = NSHostingView(
            rootView: view.frame(width: size.width, height: size.height, alignment: .topLeading)
                // On the paper the cards sit on, so a wash is a color.
                .background(WorkspaceStyle.paper)
                .environment(\._colorSchemeContrast, contrast))
        host.appearance = NSAppearance(named: name)
        host.frame = CGRect(origin: .zero, size: size)
        host.layoutSubtreeIfNeeded()
        return try #require(host.lookBitmap(scale: scale))
    }

    private func color(_ rep: NSBitmapImageRep, _ x: Int, _ y: Int) throws -> NSColor {
        try #require(rep.colorAt(x: x * rep.pixelsWide / Int(rep.size.width), y: y * rep.pixelsHigh / Int(rep.size.height)))
            .usingColorSpace(.sRGB)!
    }

    /// How far apart two colors are, channel by channel.
    private func distance(_ a: NSColor, _ b: NSColor) -> CGFloat {
        max(abs(a.redComponent - b.redComponent), abs(a.greenComponent - b.greenComponent),
            abs(a.blueComponent - b.blueComponent), abs(a.alphaComponent - b.alphaComponent))
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

    /// What the views below read for contrast.
    struct Contrast: View {
        let seen: (ColorSchemeContrast) -> Void
        @Environment(\.colorSchemeContrast) private var contrast
        var body: some View {
            let _ = seen(contrast)
            Color.clear
        }
    }

    @Test("The override is what a view reads as colorSchemeContrast", arguments: lookScales)
    func overrideIsRead(scale: Int) throws {
        var seen: [ColorSchemeContrast] = []
        _ = try bitmap(Contrast { seen.append($0) }, size: CGSize(width: 4, height: 4), .aqua, contrast: .standard, scale: scale)
        _ = try bitmap(Contrast { seen.append($0) }, size: CGSize(width: 4, height: 4), .aqua, contrast: .increased, scale: scale)
        #expect(seen.first == .standard && seen.last == .increased, "saw \(seen)")
    }

    @Test("Under Increase Contrast an approval card is outlined in amber, and not otherwise", arguments: lookScales)
    func approvalOutlined(scale: Int) throws {
        let size = CGSize(width: 320, height: 110)
        for name in Self.appearances {
            let rep = try bitmap(ApprovalCard(pending: pending, onChoose: { _ in }), size: size, name, scale: scale)
            #expect(try distance(color(rep, 0, 55), color(rep, 3, 55)) > 0.08, "no outline in \(name.rawValue)")
            let plain = try bitmap(ApprovalCard(pending: pending, onChoose: { _ in }), size: size, name, contrast: .standard, scale: scale)
            #expect(try color(plain, 0, 55) == color(plain, 3, 55), "an outline without Increase Contrast in \(name.rawValue)")
        }
    }

    @Test("Under Increase Contrast a failure card is outlined in amber", arguments: lookScales)
    func failureOutlined(scale: Int) throws {
        for name in Self.appearances {
            let rep = try bitmap(
                AgentFailureRow(failure: .notAuthenticated, agent: "Claude"), size: CGSize(width: 460, height: 150), name, scale: scale)
            // The row pads 16 around its card, so its edge is 16 in.
            #expect(try distance(color(rep, 16, 60), color(rep, 19, 60)) > 0.08, "no outline in \(name.rawValue)")
        }
    }

    @Test("A tool block waiting on you is outlined in amber and filled stronger under Increase Contrast", arguments: lookScales)
    func waitingToolBlock(scale: Int) throws {
        #expect(Tint.attentionFillOpacity(.increased) >= 2 * Fill.insetOpacity(.increased))
        #expect(Tint.attentionFillOpacity(.standard) == 0.10)
        // As `ToolRowView` draws it: the inset block, then the attention
        // surface over it while it waits.
        let waiting = Color.clear.surface(.inset, in: .card).attentionSurface(in: .card, when: true)
        let running = Color.clear.surface(.inset, in: .card).attentionSurface(in: .card, when: false)
        let size = CGSize(width: 60, height: 40)
        for name in Self.appearances {
            let (w, r) = (try bitmap(waiting, size: size, name, scale: scale), try bitmap(running, size: size, name, scale: scale))
            // Both have the inset's separator edge under Increase Contrast;
            // the waiting one's is amber.
            #expect(try distance(color(w, 0, 20), color(r, 0, 20)) > 0.08, "no amber outline in \(name.rawValue)")
            // The wash: further from a running block than the standard one is.
            let (ws, rs) = (
                try bitmap(waiting, size: size, name, contrast: .standard, scale: scale),
                try bitmap(running, size: size, name, contrast: .standard, scale: scale)
            )
            #expect(
                try distance(color(w, 30, 20), color(r, 30, 20)) > distance(color(ws, 30, 20), color(rs, 30, 20)) + 0.02,
                "the wash isn't stronger in \(name.rawValue)")
        }
    }
}
