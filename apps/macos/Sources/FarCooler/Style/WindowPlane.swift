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
    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = .sidebar
        view.blendingMode = .behindWindow
        // Dims with the window, as every sidebar does.
        view.state = .followsWindowActiveState
        return view
    }

    func updateNSView(_ view: NSVisualEffectView, context: Context) {}
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
