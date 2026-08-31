import Foundation

/// One tab in a workspace.
///
/// Two kinds, and the second one has no object behind it on the runner. Every
/// `changes.*` RPC takes a `workspace_id` and nothing else — see
/// `Session::change_set` and `Session::file_diff` in
/// `crates/client/src/session.rs`, which pass `None` where a terminal-scoped
/// call passes an id — so the diff is a fact about the worktree that this app
/// can ask for whether or not anybody ever opened a `changes` pane in it. That
/// is what lets Changes be a tab at no cost on the daemon side.
///
/// A `changes` pane the host DOES have is folded into this one by
/// `Pane.init(_:)`, and filtered out of the ribbon by `ShellFleetMap.of(_:)`.
/// Both resolve to the same `ChangesStore`, keyed by workspace on `Connection`,
/// so what has been read, what is folded, where you were and the notes you have
/// written are one review rather than one per door.
///
/// In a file of its own because the screen it used to live in is gone.
/// `WorkspaceView` was the pane host — a navigation destination that kept every
/// visited tab mounted behind the one on screen — and `ShellPaneTrack` is that
/// job now, on a swipe instead of a chip. What did not move is this type: the
/// shell's own vocabulary is `ShellTab`, which lives in AgentKit and cannot see
/// a `Terminal`, so `ShellPaneRef` carries a `Pane` beside it and this is the
/// value it carries. See `ShellPaneRef` for why that is a side table rather
/// than something encoded in a tab id.
enum Pane: Identifiable, Hashable {
    case terminal(Terminal)
    case changes

    /// The pane a terminal belongs on. A `changes` pane is the Changes tab, not
    /// a tab of its own — the alternative is two chips showing one diff.
    init(_ terminal: Terminal) {
        self = terminal.isChangesPane ? .changes : .terminal(terminal)
    }

    /// Namespaced, because a terminal id and the word "changes" are different
    /// kinds of thing and a collision between them would silently give two
    /// panes one SwiftUI identity — which is resolved by drawing one of them.
    var id: String {
        switch self {
        case .terminal(let terminal): return "terminal:\(terminal.id)"
        case .changes: return "changes"
        }
    }

    /// This tab as something worth writing down.
    ///
    /// A `Pane` holds a whole `Terminal` — a snapshot of what the daemon said
    /// when the tab was opened — and a snapshot is the wrong thing to remember,
    /// for the reason `PaneFocus` gives about its own cases. A `PaneFocus` is an
    /// id, which either still names a pane on this runner or does not, and the
    /// second answer is one `ShellFleetMap.resume` can act on.
    ///
    /// Never `.none`: that value means "nobody said", and a tab somebody moved
    /// to is the opposite of that.
    var focus: PaneFocus {
        switch self {
        case .terminal(let terminal): return .agent(terminal.id)
        case .changes: return .changes
        }
    }

    /// The terminal this tab is, where it is one. Nil for Changes, which is
    /// exactly the question `Notifier.visibleTerminal` is asking.
    var terminal: Terminal? {
        if case .terminal(let terminal) = self { return terminal }
        return nil
    }
}
