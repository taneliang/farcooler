import SwiftUI

/// The motion every list and every collapsible in the app shares (ov-104,
/// ov-101): insertions, removals, moves, and a section opening or closing,
/// all on `WorkspaceMotion.spring`, and a cross-fade alone under Reduce
/// Motion.
///
/// Moved here from `TaskBoard.swift` when every collapsible came onto it
/// (ov-101); the name stays, since the board's lists were its first users.
enum BoardMotion {
    /// The shared spring, or a cross-fade under Reduce Motion. `slowedBy`
    /// stretches it for a frame test, which reads the frames of a slowed
    /// insertion (`\.boardMotionSlowdown`); it's 1 everywhere in the app.
    static func list(reduceMotion: Bool, slowedBy: Double = 1) -> Animation {
        reduceMotion
            ? .easeInOut(duration: 0.2 * slowedBy)
            : slowedBy == 1 ? WorkspaceMotion.spring : WorkspaceMotion.spring.speed(1 / slowedBy)
    }

    /// A row, or a group, comes in fading and sliding down into its place
    /// once the rows under it have begun to make room, and goes out fading
    /// fast, before the rows under it close up over it: read in slowed frames
    /// (ov-104 review), a fade as long as the spring drew the two over each
    /// other. Only fading under Reduce Motion.
    ///
    /// It's also what a disclosure's rows do when they're siblings of its
    /// header rather than inside it: a workspace's worktrees in the sidebar's
    /// flat list, a file's lines in the diff.
    static func rowTransition(reduceMotion: Bool, slowedBy: Double = 1) -> AnyTransition {
        guard !reduceMotion else { return .opacity }
        return .asymmetric(
            insertion: .opacity.combined(with: .offset(y: -ColumnGrid.rhythm))
                .animation(.easeOut(duration: 0.2 * slowedBy).delay(0.1 * slowedBy)),
            removal: .opacity.animation(.easeOut(duration: 0.08 * slowedBy)))
    }

    /// Open or close a disclosure on the shared spring: for a disclosure
    /// whose rows are the siblings of its header in a flat list, which
    /// `CollapsibleSection` can't hold (`DisclosureButton`).
    @MainActor static func toggle(
        reduceMotion: Bool, slowedBy: Double = 1, _ change: () -> Void, completion: (() -> Void)? = nil
    ) {
        if let completion {
            withAnimation(
                list(reduceMotion: reduceMotion, slowedBy: slowedBy), completionCriteria: .removed, change,
                completion: completion)
        } else {
            withAnimation(list(reduceMotion: reduceMotion, slowedBy: slowedBy), change)
        }
    }

    /// How long a new arrival's accent wash takes to fade.
    static let highlightFade: TimeInterval = 1.5
}

extension EnvironmentValues {
    /// How much slower the board's lists, and every collapsible, move: 1,
    /// except in a frame test.
    @Entry var boardMotionSlowdown: Double = 1
}
