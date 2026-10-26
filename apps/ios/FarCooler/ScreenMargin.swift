import SwiftUI
import UIKit

// One horizontal margin for every block on a screen (ov-611): the segment
// bar, the plan's strip, the conversation's rows, the agent tray and the
// composer each sat at their own inset, and the composer was 4 pt wider than
// the rest. The margin is the system's own, read from the window's root view
// controller (`systemMinimumLayoutMargins`): 16 pt on most iPhones, 20 pt on
// the largest, as Settings and Messages lay out. A view pads by
// `@Environment(\.screenMargin)`, never by a number of its own.

extension EnvironmentValues {
    /// The screen's horizontal margin, from the layout guides. 16 until the
    /// window says otherwise.
    @Entry var screenMargin: CGFloat = ScreenMargin.fallback
    /// The window's bottom safe area, the home indicator's (34 pt on an
    /// iPhone with one, 0 on one with a Home button), for a bar that stands
    /// a fixed distance from the screen's foot (`FloatingBar`, ov-804).
    @Entry var screenFoot: CGFloat = 0
}

enum ScreenMargin {
    /// What UIKit gives an iPhone's root view in portrait, before a window
    /// has been read.
    static let fallback: CGFloat = 16
}

/// Reads the window's layout margin and its bottom safe area, and hands them
/// to everything under it.
struct ScreenMarginReader: ViewModifier {
    @State private var margin = ScreenMargin.fallback
    @State private var foot: CGFloat = 0

    func body(content: Content) -> some View {
        content
            .environment(\.screenMargin, margin)
            .environment(\.screenFoot, foot)
            .background {
                MarginProbe { margin = $0 } foot: { foot = $0 }
                    .frame(width: 0, height: 0)
                    .accessibilityHidden(true)
            }
    }
}

private struct MarginProbe: UIViewRepresentable {
    let report: (CGFloat) -> Void
    let foot: (CGFloat) -> Void

    func makeUIView(context: Context) -> ProbeView {
        let view = ProbeView()
        view.report = report
        view.foot = foot
        return view
    }

    func updateUIView(_ view: ProbeView, context: Context) {
        view.report = report
        view.foot = foot
    }

    final class ProbeView: UIView {
        var report: ((CGFloat) -> Void)?
        var foot: ((CGFloat) -> Void)?
        private var last: CGFloat?
        private var lastFoot: CGFloat?

        override func didMoveToWindow() {
            super.didMoveToWindow()
            read()
        }

        override func layoutSubviews() {
            super.layoutSubviews()
            read()
        }

        /// The root view controller's minimum margin, which is what a
        /// navigation bar's title and a list's rows line up on.
        private func read() {
            // The window's own inset, which the keyboard never changes.
            if let window, window.safeAreaInsets.bottom != lastFoot {
                let value = window.safeAreaInsets.bottom
                lastFoot = value
                let foot = foot
                DispatchQueue.main.async { foot?(value) }
            }
            guard let root = window?.rootViewController else { return }
            let margins = root.systemMinimumLayoutMargins
            let value = max(margins.leading, margins.trailing)
            guard value > 0, value != last else { return }
            last = value
            let report = report
            DispatchQueue.main.async { report?(value) }
        }
    }
}

// The navigation bar's own scheme (ov-655). A pane under the orchestrator's
// terminal puts the terminal theme's scheme on everything below it, which is
// right for its text; and wrong for the bar's buttons drawn from inside it,
// which sit on the bar's glass. On an iPad the bar is the system's, so white
// icons from a dark theme sat on light glass in light mode (contrast 1.0:1).

extension EnvironmentValues {
    /// The scheme the navigation bar is drawn in, where a screen says it;
    /// nil, and a button keeps the scheme it was given.
    @Entry var barScheme: ColorScheme?
}

/// A toolbar button drawn in the bar's scheme, not the pane's.
struct BarSchemed: ViewModifier {
    @Environment(\.barScheme) private var bar
    @Environment(\.colorScheme) private var own

    func body(content: Content) -> some View {
        content.environment(\.colorScheme, bar ?? own)
    }
}

extension View {
    /// Draw this toolbar button in the navigation bar's own scheme.
    func inBarScheme() -> some View { modifier(BarSchemed()) }
}

/// The keyboard's own motion, for a change that has to go with it (ov-804):
/// the composer's bar widening as the keyboard comes up. UIKit posts the
/// keyboard's curve as 7, a private curve no public `Animation` names, so this
/// is fitted to it: the keyboard's top read from a 60 fps recording on iOS 27
/// moved 20% of the way in the first frame, 48% by 50 ms, 76% by 100 ms and
/// 96% by 200 ms. This curve is within 1% of that at every frame. A spring
/// starts from rest, so the bar trailed the keyboard by 10 pt halfway.
enum KeyboardMotion {
    static let curve = Animation.timingCurve(0.22, 1, 0.36, 1, duration: 0.4)

    /// Nil, and the change is made at once, where the keyboard doesn't
    /// animate (a hardware keyboard's bar, a rotation's snap).
    static func animation(_ note: Notification) -> Animation? {
        let duration = note.userInfo?[UIResponder.keyboardAnimationDurationUserInfoKey] as? Double ?? 0
        return duration > 0 ? curve : nil
    }
}
