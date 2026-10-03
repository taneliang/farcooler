import AppKit
import SwiftUI

/// Whether anybody can see a window, so work that only paints can stop.
///
/// The app had no such notion, and paid for it (ov-229). Measured on a board
/// of six working agents: 45-50 % of a core while the window was on screen,
/// and the same with the app hidden. The breathing status dot kept SwiftUI
/// rendering the whole window at the display's refresh rate, and every
/// terminal kept drawing its pane on each new byte, whether anything reached
/// the screen or not.
///
/// "Visible" is AppKit's occlusion state, which is the system's own answer to
/// "is any pixel of this window on a screen": it goes false when the window
/// is entirely behind others, on another Space, minimized, or its app is
/// hidden, and when the display sleeps. Miniaturized and app-hidden are
/// checked too, because the occlusion notification can trail them.
enum WindowVisibility {
    static func isVisible(occlusion: NSWindow.OcclusionState, miniaturized: Bool, appHidden: Bool) -> Bool {
        occlusion.contains(.visible) && !miniaturized && !appHidden
    }

    @MainActor
    static func isVisible(_ window: NSWindow?) -> Bool {
        guard let window else { return false }
        #if DEBUG
            // For measuring what a window in front costs without putting one
            // in front of somebody's work: everything that pauses for a hidden
            // window runs as though this one were on screen. Debug builds only.
            if assumeVisible { return true }
        #endif
        return isVisible(
            occlusion: window.occlusionState, miniaturized: window.isMiniaturized,
            appHidden: NSApp?.isHidden ?? false)
    }
}

#if DEBUG
    extension WindowVisibility {
        static let assumeVisible = ProcessInfo.processInfo.environment["FARCOOLER_ASSUME_VISIBLE"] == "1"
    }
#endif

/// Follows one window's visibility and says when it changes.
///
/// Owned by whoever needs to know: the window's root view, through
/// `WindowVisibilityReader`, and each `TerminalRenderView`, which is an
/// `NSView` and can't read the SwiftUI environment.
@MainActor
final class WindowVisibilityWatch {
    private(set) var isVisible = false
    private var observers: [NSObjectProtocol] = []
    private weak var window: NSWindow?
    private let onChange: (Bool) -> Void

    init(onChange: @escaping (Bool) -> Void) {
        self.onChange = onChange
    }

    /// Follow `window` instead of whatever was followed before; nil follows
    /// nothing, which reads as not visible.
    func follow(_ window: NSWindow?) {
        let center = NotificationCenter.default
        observers.forEach(center.removeObserver)
        observers = []
        self.window = window
        if let window {
            let names: [(Notification.Name, AnyObject?)] = [
                (NSWindow.didChangeOcclusionStateNotification, window),
                (NSWindow.didMiniaturizeNotification, window),
                (NSWindow.didDeminiaturizeNotification, window),
                (NSApplication.didHideNotification, nil),
                (NSApplication.didUnhideNotification, nil),
            ]
            for (name, object) in names {
                observers.append(
                    center.addObserver(forName: name, object: object, queue: .main) { [weak self] _ in
                        MainActor.assumeIsolated { self?.recheck() }
                    })
            }
        }
        recheck()
    }

    /// Read the window again, and report only a change.
    func recheck() {
        let now = WindowVisibility.isVisible(window)
        guard now != isVisible else { return }
        isVisible = now
        onChange(now)
    }

    isolated deinit {
        observers.forEach(NotificationCenter.default.removeObserver)
    }
}

extension EnvironmentValues {
    /// Whether this view's window is on screen. True where nothing has said
    /// otherwise — a preview, a test, an offscreen render — so the default
    /// never freezes something a person is looking at.
    @Entry var windowVisible: Bool = true
}

/// Puts the window's visibility into the environment, for views that animate.
struct WindowVisibilityReader: ViewModifier {
    @State private var visible = true

    func body(content: Content) -> some View {
        content
            .environment(\.windowVisible, visible)
            .background(VisibilityProbe { visible = $0 })
    }
}

private struct VisibilityProbe: NSViewRepresentable {
    let onChange: (Bool) -> Void

    func makeNSView(context: Context) -> ProbeView {
        let view = ProbeView()
        view.onChange = onChange
        return view
    }

    func updateNSView(_ view: ProbeView, context: Context) {
        view.onChange = onChange
    }

    final class ProbeView: NSView {
        var onChange: ((Bool) -> Void)?
        private lazy var watch = WindowVisibilityWatch { [weak self] in self?.onChange?($0) }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            watch.follow(window)
        }

        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }
}
