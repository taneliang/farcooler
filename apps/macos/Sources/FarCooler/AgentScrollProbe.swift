import SwiftUI

/// What a test can read of a chat's scrolling, and the doors it may open
/// (ov-383).
///
/// Set in the environment by a test and nil in the app. The chat writes what
/// it decided, following or not and whether Jump to Latest is offered, with
/// the scroll geometry it decided from; and it hands over the same calls
/// its composer and its Jump to Latest make, so a test sends, fills the
/// composer and jumps through the production path without synthesizing
/// input. It never changes what the chat does.
@MainActor
final class AgentScrollProbe {
    var geometry: ScrollGeometry?
    var following = true
    var showsJump = false
    /// `AgentStream.send`, as the composer calls it.
    var send: ((String) -> Void)?
    /// Text into the composer, as Edit puts a sent message back.
    var prefill: ((String) -> Void)?
    /// What Jump to Latest does.
    var jump: (() -> Void)?

    /// How far the end of the content is under the composer, in points:
    /// zero or less when it's in view above it (`AgentSurface.tailHiddenBy`).
    var tailHiddenBy: CGFloat? { geometry.map(AgentSurface.tailHiddenBy) }
}

extension EnvironmentValues {
    @Entry var agentScrollProbe: AgentScrollProbe? = nil
}
