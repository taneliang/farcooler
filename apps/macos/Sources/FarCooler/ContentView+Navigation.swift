import AgentKit
import AppKit
import SwiftUI

/// Navigation: opening a task, a history row or a needs-you item, and moving the keyboard between the board and what it opened.
///
/// A pure move out of `ContentView.swift`, which had outgrown its size ceiling;
/// nothing here changed when it moved.
extension ContentView {
    /// Open a task from outside its navigator (the palette, a notice):
    /// its workspace, with the task beside the board and the navigator
    /// holding the keyboard, as a row clicked in it does (`chooseTask`).
    func openTask(_ id: String, host: String, workspace: String) {
        let opened = WorkspaceNavigation.openingTask(id, host: host, workspace: workspace, from: boardState)
        trail = nil
        if opened.step.keyboard == .board { boardKeyboardPending = true }
        focusColumn = opened.step.focus
        selection = opened.selection
        key(opened.step.keyboard)
    }

    /// What "ov-190" in the selection's text links to: a task on its
    /// runner's boards read so far, opened as the palette opens one.
    var taskKeyLinker: TaskKeyLinker {
        guard let host = selection?.host else { return .none }
        return .mac(
            host: host, workspaces: store.fleet.runnerWorkspaces[host] ?? [], stores: boardStores.values,
            cards: taskKeyCards, openTask: { openTask($0, host: $1, workspace: $2) })
    }

    /// A finished status's History page, in the main area (ov-103). The
    /// navigator keeps the keyboard, as it does for a task glanced at.
    func openHistory(_ status: TaskStatus, host: String, workspace: String) {
        let next = Selection.workspace(host: host, workspace: workspace, focus: .history(status))
        guard next != selection else { return }
        trail = nil
        focusColumn = false
        selection = next
    }

    /// A row in the navigator chosen: a click opens its task, or, on the
    /// task already open, goes back to the orchestrator; ↑ and ↓ (`glance`)
    /// only open. Either way the navigator keeps the keyboard, to go on.
    func chooseTask(_ id: String, host: String, workspace: String, glance: Bool) {
        let next = WorkspaceNavigation.choosing(task: id, host: host, workspace: workspace, from: selection, toggles: !glance)
        guard next != selection else { return }
        trail = nil
        let step = WorkspaceNavigation.boardStep(.choose(glance: glance), from: boardState)
        if step.keyboard == .board { boardKeyboardPending = true }
        focusColumn = step.focus
        selection = next
        key(step.keyboard)
    }

    /// What `WorkspaceNavigation.boardStep` reads: what's open, Focus, and
    /// where the keyboard is.
    var boardState: WorkspaceNavigation.BoardState {
        let scene = selection.flatMap(workspaceScene)
        return WorkspaceNavigation.BoardState(
            opened: scene?.opened != nil, focus: focusColumn, onBoard: keyboardOnBoard,
            hasNavigator: scene?.board != nil && !navigatorHidden)
    }

    /// ⌘B, View ▸ Toggle Sidebar and the title bar's button: the navigator
    /// put away or brought back (ov-178). Put away with the keyboard in
    /// it, the keyboard goes to what the main area shows.
    func toggleNavigator() {
        // Too narrow for it beside the canvas and the chat, ⌘B floats it
        // over them (ov-298), the canvas folded or not.
        if let scene = selection.flatMap(workspaceScene), scene.hasConversation, splits(scene),
            canvasSizing.navigatorFloats(detailWidth ?? 0)
        {
            navigatorFloating.toggle()
            return
        }
        navigatorHidden = NavigatorVisibility.toggled(
            navigatorHidden, hasNavigator: selection.flatMap(workspaceScene)?.board != nil)
        guard navigatorHidden, keyboardOnBoard else { return }
        keyboardOnBoard = false
        key(.main)
    }

    /// Does what a `BoardStep` says.
    func apply(_ step: WorkspaceNavigation.BoardStep) {
        focusColumn = step.focus
        key(step.keyboard)
    }

    /// The keyboard to `target`.
    func key(_ target: WorkspaceNavigation.KeyTarget) {
        switch target {
        case .board:
            // Never onto a navigator put away with ⌘B: what the main area
            // shows takes it instead.
            guard !navigatorHidden else {
                keyMain()
                return
            }
            keyboardOnBoard = true
            windowBox.window?.makeFirstResponder(nil)
            boardFocusRequest += 1
        case .main:
            keyMain()
        case .conversation:
            if let pane = shownLayouts(for: selection).first(where: { $0.column == .conversation })
                .flatMap(WorkspaceScreen.columnPane)
            {
                step(to: pane)
            } else {
                // A placeholder, a restart or a chat view: off the board, to
                // the view itself.
                keyboardOnBoard = false
                keyPane = nil
                windowBox.window?.makeFirstResponder(nil)
            }
        case .opened:
            keyOpened()
        case .unchanged:
            break
        }
    }

    /// Open a Needs You item where spec §2.5 says it lands.
    func open(_ item: NeedsYouItem) {
        lastAttention = item.key
        guard let landed = NeedsYouNavigation.landing(for: item, in: store.fleet) else {
            errorBanner = "That’s no longer on its runner."
            return
        }
        navigate(to: landed)
    }

    /// Go to `pane` wherever it lives: its workspace, its task, or its
    /// worktree (`WorkspaceSelection.landing`).
    func land(on pane: PaneRef) {
        guard let landed = WorkspaceSelection.landing(on: pane, in: store.fleet) else { return }
        navigate(to: landed, key: pane)
    }

    /// Follow the layout's focus when something else moved it.
    ///
    /// The CLI and an agent can both focus a pane, and when they do the app has
    /// to be looking at it — otherwise `farcooler layout focus` from a script
    /// draws a border around a pane whose keystrokes still go somewhere else.
    ///
    /// Within one layout only, and now that is the whole of what it does. The
    /// last condition — the selected terminal being in the layout that moved —
    /// is what keeps this from flip-flopping: the app assumes a focus locally
    /// (`DaemonClient.assumeFocus`) and the runner's confirmation of the
    /// PREVIOUS focus can still be in flight, so a version that followed across
    /// layouts would follow that stale answer straight back. The cost is that
    /// `layout focus` aimed at another layout no longer brings it on screen —
    /// the app draws the layout holding the SELECTED terminal now, so it stays
    /// where you left it. Selection and screen agree, which they did not before;
    /// the runner and the app disagree about which layout is at the front, which
    /// nothing on screen claims either way.
    func followLayoutFocus() {
        guard let pane = selectedPane,
            let worktree = worktree(host: pane.host, id: pane.worktree),
            let group = store.client(for: worktree)?.activeGroup(pane.worktree),
            let focused = group.focused,
            focused != pane.terminal,
            group.terminals.contains(pane.terminal)
        else { return }
        focus(PaneRef(host: pane.host, worktree: pane.worktree, terminal: focused))
    }
}
