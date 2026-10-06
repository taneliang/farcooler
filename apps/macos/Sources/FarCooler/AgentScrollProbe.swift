import SwiftUI

/// What a test can read of a chat's scrolling, and the doors it may open
/// (ov-383).
///
/// Set in the environment by a test and nil in the app. The chat writes what
/// it decided, following or not and whether Jump to Latest is offered, with
/// the scroll geometry it decided from, and where its last row ends; and it
/// hands over the calls its composer makes, so a test sends and fills the
/// composer through the production path. Jump to Latest a test clicks. It
/// never changes what the chat does.
@MainActor
final class AgentScrollProbe {
    var geometry: ScrollGeometry?
    /// The transcript's last row, in the scroll view's visible coordinates.
    var lastRow: CGRect?
    var following = true
    var showsJump = false
    /// `AgentStream.send`, as the composer calls it.
    var send: ((String) -> Void)?
    /// Text into the composer, as Edit puts a sent message back.
    var prefill: ((String) -> Void)?

    /// How far the end of the content is under the composer, in points:
    /// zero or less when it's in view above it (`AgentSurface.tailHiddenBy`).
    var tailHiddenBy: CGFloat? { geometry.map(AgentSurface.tailHiddenBy) }

    /// The space between the last row's bottom and the composer's top edge.
    var clearance: CGFloat? {
        guard let geometry, let lastRow else { return nil }
        return geometry.containerSize.height - lastRow.maxY
    }
}

/// Reports the row it's on to `probe`, when there is one. A modifier so the
/// app, with no probe, gets the row with nothing added.
struct LastRowProbe: ViewModifier {
    let probe: AgentScrollProbe?

    func body(content: Content) -> some View {
        if let probe {
            content.onGeometryChange(for: CGRect.self) { $0.frame(in: .scrollView(axis: .vertical)) } action: {
                probe.lastRow = $0
            }
        } else {
            content
        }
    }
}

extension EnvironmentValues {
    @Entry var agentScrollProbe: AgentScrollProbe? = nil
}
