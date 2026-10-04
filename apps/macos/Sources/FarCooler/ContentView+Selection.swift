import AgentKit
import AppKit
import SwiftUI

/// Selection: which worktree a keystroke acts on, stepping through terminals, and healing a selection whose worktree went away.
///
/// A pure move out of `ContentView.swift`, which had outgrown its size ceiling;
/// nothing here changed when it moved.
extension ContentView {
    /// The worktree the detail pane is actually showing.
    ///
    /// Not `currentWorktree` by name, though the two now compute the same
    /// value: `currentWorktree`'s own fallback to the first worktree in the
    /// fleet is gone, removed for the same reason this property never had
    /// one — with nothing selected, the placeholder is on screen, and
    /// offering to open a worktree the window is not showing would be the
    /// control lying about what it points at. Kept as a separate property so
    /// each name still reads as what it answers: this one, what the detail
    /// pane draws; `currentWorktree`, what a keystroke acts on.
    var detailWorktree: Worktree? {
        currentWorktree
    }

    /// The worktree the selection is in — nil when nothing is selected.
    ///
    /// Used to be "or the first one": with nothing selected, that first
    /// worktree could belong to ANY runner in the fleet, chosen by nothing
    /// more meaningful than merge order. `tileTarget` and ⌘T both read this
    /// to decide what a keystroke acts on, and a fallback here meant a ⌃B
    /// command issued while the detail pane was blank still landed — split,
    /// break, preset, cycle, zoom, focus, a new terminal — on whichever
    /// runner happened to own that first row. Wrong-runner routing is the
    /// one failure this feature must never produce, so with no selection
    /// there is now no target, and `tileTarget`'s and `.newTerminal`'s own
    /// `guard`/`if let` already do nothing rather than guess.
    var currentWorktree: Worktree? {
        // The key pane's, which is on screen by construction. A board alone
        // is a workspace's, not a worktree's: a keystroke that acts on "the
        // current worktree" has nothing to act on there.
        if let pane = selectedPane { return worktree(host: pane.host, id: pane.worktree) }
        if let (host, id) = Self.openedWhole(selection) { return worktree(host: host, id: id) }
        return nil
    }

    func step(by offset: Int) {
        let ordered = allTerminals
        guard !ordered.isEmpty else { return }
        let current = ordered.firstIndex { $0 == selectedPane } ?? 0
        // Wraps, because a list you can walk off the end of makes you look.
        let next = (current + offset + ordered.count) % ordered.count
        step(to: ordered[next])
    }

    /// Move the selection off a terminal, or a worktree, that has gone.
    ///
    /// Prefers to stay where the user was looking: another terminal in the same
    /// worktree, whatever wants attention first, then anything running. Only
    /// falls back to the worktree itself when the worktree is empty, and to a
    /// sibling worktree on the same runner when the worktree itself is gone.
    ///
    /// The one rule for every way a terminal or a worktree can disappear,
    /// closing one here with ⌘W included. See `healed(_:in:was:)`, which is the
    /// rule itself. `previous` is the fleet before the change, which is the only
    /// place a removed worktree's repository can still be read.
    func healSelection(previous: [Worktree] = []) {
        let next = Self.healed(
            selection, in: store.fleet.worktrees, was: previous, workspaces: store.fleet.runnerWorkspaces)
        if next != selection { selection = next }
    }

    /// Where a selection goes when what it points at is gone, or the same
    /// selection when it is not.
    ///
    /// Static and free of the view so it can be tested; `healSelection` holds
    /// only the assignment.
    ///
    /// - **Never leaves the worktree while the worktree is there.** A closed
    ///   terminal's neighbor is in the worktree you were working in, not
    ///   wherever the fleet happens to list a running terminal first. The
    ///   phones keep the same promise their own way, clamping to the
    ///   neighboring tab (`ShellFleet.reseat`).
    /// - **Never leaves the runner.** When the worktree itself is gone, a
    ///   terminal selection and a worktree selection both land on a sibling
    ///   worktree on the same runner (see `sibling(of:host:in:was:)`), or on
    ///   nothing. It used to take the first worktree in the merged fleet,
    ///   which is often another runner's, and it left a selected worktree's
    ///   id in place after the worktree was gone.
    nonisolated static func healed(
        _ selection: Selection?, in worktrees: [Worktree], was previous: [Worktree] = [],
        workspaces: [String: [WorkspaceSummary]] = [:]
    ) -> Selection? {
        let host: String
        let worktreeID: String
        let terminalID: String?
        switch selection {
        case nil: return nil
        // Needs You is every runner's, and has nothing to heal.
        case .needsYou: return selection
        // A workspace with nothing opened has nothing to heal either: a
        // runner that loses it draws the column's sentence until you choose.
        // A task is its own view's to say it's gone.
        case .workspace(_, _, nil), .workspace(_, _, .task), .workspace(_, _, .history), .workspace(_, _, .plan):
            return selection
        case .workspace(let h, _, .worktree(let w, let t)): (host, worktreeID, terminalID) = (h, w, t)
        case .looseWorktree(let h, let w, let t): (host, worktreeID, terminalID) = (h, w, t)
        }
        /// The same selection, opening `worktree` with `terminal` in it.
        func with(_ worktree: String, terminal: String?) -> Selection {
            switch selection {
            case .workspace(let h, let id, _)?: return .workspace(host: h, workspace: id, focus: .worktree(worktree, terminal: terminal))
            default: return .looseWorktree(host: host, worktree: worktree, terminal: terminal)
            }
        }
        guard
            let worktree = worktrees.first(where: {
                ($0.host ?? "") == host && $0.id == worktreeID
            })
        else {
            // Gone: a workspace's column closes, back to the workspace; a
            // loose worktree lands on a sibling on its runner.
            if case .workspace(let h, let id, _)? = selection { return .workspace(host: h, workspace: id, focus: nil) }
            return sibling(of: worktreeID, host: host, in: worktrees, was: previous, workspaces: workspaces)
        }
        guard let terminalID, !worktree.terminals.contains(where: { $0.id == terminalID })
        else {
            return selection
        }

        let candidates = worktree.terminals
        let next = candidates.first(where: { $0.status.wantsAttention })
            ?? candidates.first(where: { StateKind.parse($0.state) == .running })
            ?? candidates.first
        return with(worktreeID, terminal: next?.id)
    }

    /// The worktree to land on when `worktreeID` is gone: one on the same
    /// runner, in the same repository if `previous` still says which that was,
    /// and one that isn't hidden before a hidden one. Nil when the
    /// runner has none left, because landing on another runner is the app
    /// moving you somewhere you never asked to go.
    nonisolated static func sibling(
        of worktreeID: String, host: String, in worktrees: [Worktree],
        was previous: [Worktree], workspaces: [String: [WorkspaceSummary]] = [:]
    ) -> Selection? {
        let repository = previous.first {
            ($0.host ?? "") == host && $0.id == worktreeID
        }?.repository
        let sameRunner = worktrees.filter { ($0.host ?? "") == host && $0.id != worktreeID }
        let shown = sameRunner.filter { !$0.isHidden }
        let next =
            shown.first(where: { repository != nil && $0.repository == repository })
            ?? shown.first
            ?? sameRunner.first
        // Opened as its row would open it: under the workspace that owns it,
        // and loose only when none does.
        var fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: worktrees, branchPrefix: nil)
        fleet.runnerWorkspaces = workspaces
        return next.map { opening($0, terminal: nil, in: fleet) }
    }

    func selectTerminal(at index: Int) {
        let ordered = allTerminals
        guard index >= 0, index < ordered.count else { return }
        step(to: ordered[index])
    }

    /// Put the keyboard in a pane on screen, and tmux's focus with it.
    func step(to pane: PaneRef) {
        focus(pane)
        guard let worktree = worktree(host: pane.host, id: pane.worktree),
            let rect = store.client(for: worktree)?.group(holding: pane.terminal, in: pane.worktree)?.pane(pane.terminal)
        else { return }
        Task { await act(.arrange, on: worktree) { c in await c.focusPane(rect.short, in: worktree) } }
    }
}
