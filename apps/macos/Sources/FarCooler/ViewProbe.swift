import SwiftUI

/// A view a test looks for, by its accessibility identifier, and where it
/// was drawn (ov-177).
///
/// The accessibility tree an `NSHostingView` offers is empty until an
/// assistive app asks for it, so a test drawing a view offscreen can't find
/// what it drew by its identifier there. Under `gridProbing`, which only a
/// test sets, each view marked `identified(_:)` reports itself here as well;
/// the app pays nothing.
struct ProbedView {
    let id: String
    let bounds: Anchor<CGRect>
}

struct ProbedViewsKey: PreferenceKey {
    static let defaultValue: [ProbedView] = []
    static func reduce(value: inout [ProbedView], nextValue: () -> [ProbedView]) {
        value += nextValue()
    }
}

private struct IdentifiedModifier: ViewModifier {
    let id: String
    /// Whether it's the view's accessibility identifier too: not for a
    /// region whose controls carry their own (`probed(_:)`).
    var labels = true
    @Environment(\.gridProbing) private var probing

    func body(content: Content) -> some View {
        if probing {
            // Added to what's inside it, not set: `anchorPreference` would
            // hide every probed view within this one, as a header's probe
            // hid its own button.
            labeled(
                content.transformAnchorPreference(key: ProbedViewsKey.self, value: .bounds) {
                    $0.append(ProbedView(id: id, bounds: $1))
                })
        } else {
            labeled(content)
        }
    }

    @ViewBuilder private func labeled(_ view: some View) -> some View {
        if labels { view.accessibilityIdentifier(id) } else { view }
    }
}

extension View {
    /// `accessibilityIdentifier(id)`, and under `gridProbing`, reported to a
    /// test with where it was drawn (`ProbedViewsKey`).
    func identified(_ id: String) -> some View {
        modifier(IdentifiedModifier(id: id))
    }

    /// Reported to a test as `identified(_:)` is, and nothing else: for a
    /// region a test points at, such as a section's header row, whose own
    /// controls carry the accessibility identifiers.
    func probed(_ id: String) -> some View {
        modifier(IdentifiedModifier(id: id, labels: false))
    }
}

/// Under `gridProbing`, a zero-height probe hung on the view's first text
/// baseline, reported as `identified(_:)` is, so a test can read where a
/// label's or a symbol's baseline was drawn (ov-290). Nothing otherwise.
private struct BaselineProbe: ViewModifier {
    let id: String?
    @Environment(\.gridProbing) private var probing

    func body(content: Content) -> some View {
        if probing, let id {
            content.background(alignment: Alignment(horizontal: .leading, vertical: .firstTextBaseline)) {
                // style-exempt: an invisible test probe on the baseline, not a rule
                Color.clear.frame(width: 1, height: 0).probed(id)
            }
        } else {
            content
        }
    }
}

extension View {
    /// Reported to a test at its first text baseline (`BaselineProbe`).
    func baselineProbed(_ id: String?) -> some View {
        modifier(BaselineProbe(id: id))
    }
}
