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
}

enum ScreenMargin {
    /// What UIKit gives an iPhone's root view in portrait, before a window
    /// has been read.
    static let fallback: CGFloat = 16
}

/// Reads the window's layout margin and hands it to everything under it.
struct ScreenMarginReader: ViewModifier {
    @State private var margin = ScreenMargin.fallback

    func body(content: Content) -> some View {
        content
            .environment(\.screenMargin, margin)
            .background {
                MarginProbe { margin = $0 }
                    .frame(width: 0, height: 0)
                    .accessibilityHidden(true)
            }
    }
}

private struct MarginProbe: UIViewRepresentable {
    let report: (CGFloat) -> Void

    func makeUIView(context: Context) -> ProbeView {
        let view = ProbeView()
        view.report = report
        return view
    }

    func updateUIView(_ view: ProbeView, context: Context) { view.report = report }

    final class ProbeView: UIView {
        var report: ((CGFloat) -> Void)?
        private var last: CGFloat?

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
