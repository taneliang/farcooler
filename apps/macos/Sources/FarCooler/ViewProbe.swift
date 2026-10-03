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
    @Environment(\.gridProbing) private var probing

    func body(content: Content) -> some View {
        if probing {
            content
                .anchorPreference(key: ProbedViewsKey.self, value: .bounds) { [ProbedView(id: id, bounds: $0)] }
                .accessibilityIdentifier(id)
        } else {
            content.accessibilityIdentifier(id)
        }
    }
}

extension View {
    /// `accessibilityIdentifier(id)`, and under `gridProbing`, reported to a
    /// test with where it was drawn (`ProbedViewsKey`).
    func identified(_ id: String) -> some View {
        modifier(IdentifiedModifier(id: id))
    }
}
