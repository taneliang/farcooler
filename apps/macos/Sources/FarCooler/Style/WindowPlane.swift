import AppKit
import SwiftUI

/// The frosted plane behind the window's chrome (ov-220, ov-216): the system
/// sidebar material, blending with what is behind the window. The navigator,
/// the gutters around the cards and the empty states draw nothing of their own
/// over it; the work sits on it as opaque paper (`Surface.content`), so text
/// never reads over the wallpaper.
///
/// An `NSVisualEffectView` rather than `containerBackground(for: .window)`,
/// which only a `NavigationSplitView` or `NavigationStack` container hands to
/// its window: this window's columns are our own, and the AppKit view says what
/// it is in one place. Reduce Transparency and Increase Contrast are the
/// system's to honor: the view draws an opaque fill under either.
struct WindowPlane: NSViewRepresentable {
    /// The one material of the window's chrome (ov-289): the toolbar, the
    /// navigator, the gutters and the headers all show this, because the
    /// titlebar draws none of its own (`MainWindowChrome.unify`).
    static let material: NSVisualEffectView.Material = .sidebar

    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = Self.material
        view.blendingMode = .behindWindow
        // Dims with the window, as every sidebar does.
        view.state = .followsWindowActiveState
        return view
    }

    func updateNSView(_ view: NSVisualEffectView, context: Context) {}
}

/// Text on the plane that isn't the primary label (ov-289): the system's
/// secondary and tertiary labels are 3.95:1 and lower in light, and the plane
/// is frosted, so a bright or dark wallpaper moves the ground under them.
/// The text color (the primary label without its 85% alpha) at 70% is 5:1 or better on the plane's fallback and on a
/// ground pushed 20% toward the wallpaper's worst case, in light and dark
/// (`SurfaceTests`). One level: a hierarchy among the plane's texts is by size
/// and weight, as a source list's is.
enum SidebarInk {
    static let opacity: Double = 0.7
    static let secondary = Color(nsColor: .textColor).opacity(opacity)  // style-exempt: the plane's one readable secondary ink
}

/// The space between a card and the window's edge, and between cards.
///
/// Concentric with the window: its corner radius minus `Radius.medium`. The
/// corner is about 16 pt on macOS 27, which makes 6, and about 25 pt on macOS
/// 26, which would make 15, so 26 keeps the 10 it has always had, with the card
/// clear of the curve. `WindowCornerTests` measures a real window and fails when
/// an OS changes the corner again.
enum Gutter {
    static var window: CGFloat { window(osMajor: ProcessInfo.processInfo.operatingSystemVersion.majorVersion) }

    /// The gutter on macOS `osMajor`.
    static func window(osMajor: Int) -> CGFloat { osMajor >= 27 ? 6 : 10 }
}
