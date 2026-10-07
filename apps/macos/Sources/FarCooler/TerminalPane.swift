import AgentKit
import SwiftUI

/// The terminal detail pane: a header of facts, the live surface, a hint bar.
/// The terminal.
///
/// No header bar and no footer hint. Both were chrome explaining what the
/// sidebar already shows and what a terminal obviously is — a title repeating
/// the selected row, a status repeating its dot, and a permanent sentence about
/// how typing works. The content is a terminal; the window title says which
/// one; everything else was in the way of it.
struct TerminalPane: View {
    @ObservedObject private var preferences = Preferences.shared

    let terminal: Terminal
    let worktree: Worktree
    let binary: String?
    let environment: [String: String]
    let hostArguments: [String]
    /// Which link the panes below were opened on, so a runner that dropped
    /// and came back gets fresh streams instead of frozen ones. See
    /// `DaemonClient.linkGeneration`.
    let linkGeneration: Int
    /// Why this pane's runner cannot be acted on, or nil if it can — passed
    /// straight through to `AgentSurface`, which is the one branch below
    /// that mutates outside `onAction`'s own `act(on:)` gate.
    let refusal: () -> String?
    let onGeometry: (Int, Int) async -> Void
    let onSearchFiles: (String) async -> [String]
    let onAction: (TerminalAction) -> Void
    /// Whether this pane takes the keyboard when it appears. Not beside a
    /// tiled column that holds the key pane: typed keys would go here while
    /// ⌃B and ⌘W acted there.
    var hasKeyboard = true

    /// Whether to draw a terminal here at all.
    ///
    /// `.unknown` counts, and that is the point of it. It means the daemon could
    /// not read this runner's pane inventory on this tick — not that this pane
    /// died — and the byte stream feeding this view is a channel of its own that
    /// a failed `list-panes` does not touch.
    ///
    /// Treating it as not-live tore the surface down, killed a stream that was
    /// working, and replaced a terminal somebody was reading with "no running
    /// session", over a hiccup usually finished before the view could rebuild.
    /// The row's own indicator still says "Not answering", so nothing here
    /// pretends the state is known; it just does not throw the pane away over
    /// it.
    private var isLive: Bool {
        let kind = StateKind.parse(terminal.state)
        return kind == .running || kind == .starting || kind == .unknown
    }

    var body: some View {
        VStack(spacing: 0) {
            // A draft held behind a dialog in this pane (ov-385).
            HeldDraftBar(hold: terminal.draftHold) { onAction(.withdrawDraft) }
            surface
        }
        // A card on the canvas, exactly as a tiled pane is — so one terminal and
        // four are the same object at different counts, and neither has to
        // pretend to have the window's corner. See `Pane`.
        .paneCard()
        .paneCanvas()
        // The prefix works here too — ⌃B t is how a worktree gets tiled at all —
        // so the hint has to be visible here.
        .prefixHint()
        // The window's own title bar, which macOS already draws. Free, native,
        // and it costs the content no vertical space.
        .navigationTitle(worktree.windowTitle)
        .navigationSubtitle(worktree.windowSubtitle)
    }

    /// The pane's content: its chat, its terminal, or what's left of one.
    @ViewBuilder
    private var surface: some View {
        Group {
            if isLive, terminal.isAgentPane {
                // Same rectangle, same lifecycle, a chat drawn into it instead
                // of a VT grid. See `AgentSurface`'s own doc comment for why
                // it still owes `onGeometry` an honest answer even though it
                // draws no grid of its own.
                AgentSurface(
                    terminal: terminal,
                    binary: binary,
                    environment: environment,
                    hostArguments: hostArguments,
                    linkGeneration: linkGeneration,
                    refusal: refusal,
                    // One pane, so it always owns the keyboard.
                    isFocused: hasKeyboard,
                    searchFiles: onSearchFiles,
                    onResize: onGeometry
                )
                // Identity includes the PANE MODE, not just the terminal.
                //
                // A mode switch respawns the pane: new process, new epoch, new
                // stream and input channel. With an id that ignores mode,
                // SwiftUI reuses the existing view, which stays bound to the
                // process that is gone — so the terminal comes back black and
                // swallows every keystroke, and only switching layouts (which
                // rebuilds everything) appears to fix it.
                .id("\(terminal.id)#\(terminal.paneMode ?? "terminal")")
            } else if isLive {
                // With its native view beside it, for claude (ov-372).
                NativeSwitch(terminal: terminal, target: hostArguments.joined(separator: " "), isFocused: hasKeyboard) { focused in
                    TerminalSurface(
                        terminal: terminal.short,
                        binary: binary,
                        environment: environment,
                        hostArguments: hostArguments,
                        linkGeneration: linkGeneration,
                        onResize: onGeometry,
                        fontRevision: preferences.revision,
                        // One pane, so it always owns the keyboard, unless
                        // its native view is showing.
                        isFocused: focused
                    )
                }
                // Identity includes the PANE MODE, not just the terminal.
                //
                // A mode switch respawns the pane: new process, new epoch, new
                // stream and input channel. With an id that ignores mode,
                // SwiftUI reuses the existing view, which stays bound to the
                // process that is gone — so the terminal comes back black and
                // swallows every keystroke, and only switching layouts (which
                // rebuilds everything) appears to fix it.
                .id("\(terminal.id)#\(terminal.paneMode ?? "terminal")")
            } else {
                inactive
            }
        }
    }

    /// The only state left that needs saying out loud: a terminal with no
    /// running pane. Its own page, with why, and Restart and Dismiss
    /// (ov-191). See `LostTerminalPage`.
    private var inactive: some View {
        LostTerminalPage(terminal: terminal, worktree: worktree, hasKeyboard: hasKeyboard, onAction: onAction)
    }
}
