import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// What the window plane and the paper on it draw (ov-220): the plane draws
/// nothing of its own, and work is never translucent.
@MainActor
struct SurfaceTests {
    /// The alpha at `point` of `view` drawn into a bitmap under `appearance`.
    private func alpha<V: View>(of view: V, at point: CGPoint, size: CGSize, _ name: NSAppearance.Name = .aqua) throws -> CGFloat {
        try pixel(of: view, at: point, size: size, name).alphaComponent
    }

    /// The color at `point` of `view` drawn into a bitmap under `name`.
    private func pixel<V: View>(of view: V, at point: CGPoint, size: CGSize, _ name: NSAppearance.Name = .aqua) throws -> NSColor {
        let host = NSHostingView(rootView: view.frame(width: size.width, height: size.height))
        host.appearance = NSAppearance(named: name)
        host.frame = CGRect(origin: .zero, size: size)
        host.layoutSubtreeIfNeeded()
        let rep = try #require(host.bitmapImageRepForCachingDisplay(in: host.bounds))
        host.cacheDisplay(in: host.bounds, to: rep)
        return try #require(rep.colorAt(x: Int(point.x), y: Int(point.y)).flatMap { $0.usingColorSpace(.sRGB) })
    }

    private let size = CGSize(width: 80, height: 60)
    private let middle = CGPoint(x: 40, y: 30)

    @Test("An empty window-level surface draws nothing")
    func windowDrawsNothing() throws {
        let view = Color.clear.surface(.window, in: .card)
        #expect(try alpha(of: view, at: middle, size: size) == 0)
    }

    @Test("Content paper is opaque in both appearances, with the theme's document color too")
    func contentIsNeverTranslucent() throws {
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let plain = Color.clear.surface(.content, in: .card)
            let themed = Color.clear.surface(.content, in: .card, fill: WorkspaceStyle.document)
            #expect(try alpha(of: plain, at: middle, size: size, name) == 1)
            #expect(try alpha(of: themed, at: middle, size: size, name) == 1)
        }
    }

    /// A bitmap can't blur what's behind a window, so the plane draws its own
    /// fallback there; the navigator is bare when its pixels are the plane's.
    @Test("The navigator's column draws nothing over the plane")
    func theNavigatorLeavesThePlaneBare() throws {
        let window = WorkspaceView(
            opened: "task" as String?, hasConversation: true, cell: 8, focused: false,
            navigatorWidth: .constant(280),
            conversation: { Color.clear }, navigator: { Color.clear }, breadcrumb: { _ in Color.clear },
            detail: { _, _ in Color.clear.background(WorkspaceStyle.document) })
        let wide = CGSize(width: 900, height: 400)
        let there = CGPoint(x: 100, y: 200)
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            let plane = try pixel(of: WindowPlane(), at: there, size: wide, name)
            let navigator = try pixel(of: window, at: there, size: wide, name)
            #expect(navigator == plane, "something is painted over the plane in \(name.rawValue)")
        }
        // And the paper beside it is opaque.
        #expect(try alpha(of: window, at: CGPoint(x: 700, y: 300), size: wide) == 1)
    }

    @Test("A pane's gutter shows the plane")
    func theGutterLeavesThePlaneBare() throws {
        let canvas = Color.clear.paneCanvas()
        #expect(try alpha(of: canvas, at: CGPoint(x: 2, y: 2), size: size) == 0)
    }

    /// WCAG relative luminance of an sRGB color.
    private func luminance(_ color: NSColor) -> CGFloat {
        let c = color.usingColorSpace(.sRGB) ?? color
        func linear(_ v: CGFloat) -> CGFloat { v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4) }
        return 0.2126 * linear(c.redComponent) + 0.7152 * linear(c.greenComponent) + 0.0722 * linear(c.blueComponent)
    }

    /// `ink` laid over `ground`, as a contrast ratio.
    private func contrast(_ ink: NSColor, on ground: NSColor) -> CGFloat {
        let g = ground.usingColorSpace(.sRGB) ?? ground
        let i = ink.usingColorSpace(.sRGB) ?? ink
        let a = i.alphaComponent
        let laid = NSColor(
            srgbRed: i.redComponent * a + g.redComponent * (1 - a), green: i.greenComponent * a + g.greenComponent * (1 - a),
            blue: i.blueComponent * a + g.blueComponent * (1 - a), alpha: 1)
        let (hi, lo) = (max(luminance(laid), luminance(g)), min(luminance(laid), luminance(g)))
        return (hi + 0.05) / (lo + 0.05)
    }

    /// The text the plane and the paper carry is `.primary`, 12:1 or better
    /// in either appearance, on the paper and on the plane's own fallback
    /// (what Reduce Transparency draws), so frost never costs a body line its
    /// 4.5:1. (The system's secondary label is 3.95:1 in light, as it was on the
    /// old canvas: not this lane's to change.)
    @Test("Primary text keeps 4.5:1 on the plane and the paper in light and dark")
    func primaryTextReadsOnThePlaneAndThePaper() {
        for name in [NSAppearance.Name.aqua, .darkAqua] {
            NSAppearance(named: name)!.performAsCurrentDrawingAppearance {
                #expect(contrast(.labelColor, on: .windowBackgroundColor) >= 4.5)
                #expect(contrast(.labelColor, on: WorkspaceStyle.documentNS) >= 4.5)
            }
        }
    }
}
