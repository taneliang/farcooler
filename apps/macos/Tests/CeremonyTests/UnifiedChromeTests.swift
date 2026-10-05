import AgentKit
import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The window's chrome is one surface (ov-289): the toolbar, the navigator,
/// the gutters and the headers all show the plane's one material, and the
/// text on it reads in light and dark.
@MainActor
struct UnifiedChromeTests {
    /// Every `NSVisualEffectView` under `view`.
    private func effectViews(in view: NSView) -> [NSVisualEffectView] {
        ((view as? NSVisualEffectView).map { [$0] } ?? []) + view.subviews.flatMap { effectViews(in: $0) }
    }

    /// The titlebar draws nothing of its own, so the plane behind the toolbar
    /// is what shows under it: goes red if the window gets its own titlebar
    /// backdrop or a rule back.
    @Test("The titlebar is transparent with no rule, so the toolbar shows the plane")
    func theToolbarShowsThePlane() async throws {
        let window = try await TitleBarHarness.window(Color.clear.mainWindowChrome(), width: 900)
        defer { window.close() }
        #expect(window.titlebarAppearsTransparent)
        #expect(window.titlebarSeparatorStyle == .none)
        #expect(window.toolbarStyle == MainWindowChrome.toolbarStyle)
    }

    /// One material, whichever surface asks: the plane the workspace draws
    /// and the one the Needs You page draws are the same view, and it is the
    /// system sidebar's, never a second material of a view's own.
    @Test("The window draws one frosted plane, in the pinned material")
    func theWindowHasOnePlaneMaterial() async throws {
        let plane = WindowPlane()
        let window = try await TitleBarHarness.window(plane, width: 400)
        defer { window.close() }
        let views = effectViews(in: try #require(window.contentView))
        #expect(views.count == 1)
        #expect(views.first?.material == WindowPlane.material)
        #expect(views.first?.blendingMode == .behindWindow)
        #expect(WindowPlane.material == .sidebar)
        // The workspace's window carries exactly that plane and nothing else
        // frosted: no second material over the navigator or the headers.
        let workspace = WorkspaceView(
            opened: "task" as String?, hasConversation: true, cell: 8, focused: false,
            navigatorWidth: .constant(280),
            conversation: { Color.clear }, navigator: { Color.clear }, breadcrumb: { _ in Color.clear },
            detail: { _, _ in Color.clear })
        let second = try await TitleBarHarness.window(workspace, width: 900)
        defer { second.close() }
        let found = effectViews(in: try #require(second.contentView))
        #expect(found.count == 1, "\(found.count) frosted views in the workspace")
        #expect(found.first?.material == WindowPlane.material)
    }

    private func luminance(_ c: NSColor) -> CGFloat {
        let c = c.usingColorSpace(.sRGB) ?? c
        func lin(_ v: CGFloat) -> CGFloat { v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4) }
        return 0.2126 * lin(c.redComponent) + 0.7152 * lin(c.greenComponent) + 0.0722 * lin(c.blueComponent)
    }

    private func blend(_ ink: NSColor, over ground: NSColor) -> NSColor {
        let g = ground.usingColorSpace(.sRGB) ?? ground
        let i = ink.usingColorSpace(.sRGB) ?? ink
        let a = i.alphaComponent
        return NSColor(
            srgbRed: i.redComponent * a + g.redComponent * (1 - a), green: i.greenComponent * a + g.greenComponent * (1 - a),
            blue: i.blueComponent * a + g.blueComponent * (1 - a), alpha: 1)
    }

    private func contrast(_ a: NSColor, _ b: NSColor) -> CGFloat {
        let (hi, lo) = (max(luminance(a), luminance(b)), min(luminance(a), luminance(b)))
        return (hi + 0.05) / (lo + 0.05)
    }

    /// The plane's fallback ground, and the worst a wallpaper does to the
    /// frost: the ground pushed 20% toward the other end (a dark wallpaper
    /// under light frost, a bright one under dark frost).
    @Test("The plane's secondary ink keeps 4.5:1 in light and dark, over a bright or dark wallpaper")
    func secondaryInkReadsOnThePlane() {
        for (name, other) in [(NSAppearance.Name.aqua, NSColor.black), (.darkAqua, .white)] {
            NSAppearance(named: name)!.performAsCurrentDrawingAppearance {
                let fallback = NSColor.windowBackgroundColor.usingColorSpace(.sRGB)!
                let pushed = blend(other.withAlphaComponent(0.2), over: fallback)
                let ink = NSColor(SidebarInk.secondary).usingColorSpace(.sRGB)!
                for ground in [fallback, pushed] {
                    #expect(contrast(blend(ink, over: ground), ground) >= 4.5, "\(name.rawValue) \(ground)")
                }
            }
        }
    }
}
