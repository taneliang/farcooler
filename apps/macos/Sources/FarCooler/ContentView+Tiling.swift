import AgentKit
import AppKit
import SwiftUI

/// Tiling: every pane move, resize, mode switch and focus change that goes through tmux.
///
/// A pure move out of `ContentView.swift`, which had outgrown its size ceiling;
/// nothing here changed when it moved.
extension ContentView {
    /// Carry out a `⌃B`-prefixed command.
    ///
    /// Every one of them is a single `layout` call whose reply is the worktree's
    /// whole layout, so there is nothing to reconcile here: tmux decides what a
    /// zoom or a split means, and it decides the same way for this app, for the
    /// CLI and for an agent driving the CLI.
    ///
    /// Each names the layout on screen (`shown`), never leaving the runner to
    /// pick one. The runner's pick for the main checkout is never an
    /// orchestrator's window, so the orchestrator's row has to name its own,
    /// or its ⌃B z would zoom the checkout's.
    ///
    /// This file no longer contributes geometry. Directional focus used to be
    /// worked out here from a recomputed arrangement; it is now read off the
    /// rectangles tmux reported, which is the only copy.
    func tile(_ command: TileCommand) async {
        guard let worktree = tileTarget else { return }
        let screen = onScreen(in: worktree)
        let group = screen?.group
        let shown = group?.id
        /// The pane a keystroke acts on: the selected one, else whatever tmux says
        /// is focused.
        let here: PaneRect? = {
            if let id = selectedPane?.terminal, let pane = group?.pane(id) { return pane }
            return group?.panes.first(where: \.focused)
        }()
        // The orchestrator is one pane (ov-78): with the keyboard in its
        // column, what would add a pane there opens a shell in the main
        // checkout instead, which the workspace opens beside its board.
        if WorkspaceScreen.opensShellInstead(command, key: selectedPane, in: self.shown) {
            await openShell(besideOrchestratorIn: worktree)
            return
        }

        switch command {
        case .zoom:
            await act(.arrange, on: worktree) { c in await c.zoomPane(nil, in: worktree, layout: shown) }

        case .focusNext:
            await act(.arrange, on: worktree) { c in
                await c.focusPane(step: "--next", in: worktree, layout: shown)
            }
        case .focusPrevious:
            await act(.arrange, on: worktree) { c in
                await c.focusPane(step: "--prev", in: worktree, layout: shown)
            }

        case .focus(let direction):
            guard let group, let from = here,
                let next = group.neighbour(of: from.id, direction)
            else { return }
            await act(.arrange, on: worktree) { c in await c.focusPane(next.short, in: worktree) }

        case .focusIndex(let n):
            // Counted in the layout on screen. See `pane(numbered:in:)`.
            guard let pane = Self.pane(numbered: n, in: group) else { return }
            await act(.arrange, on: worktree) { c in await c.focusPane(pane.short, in: worktree) }

        case .cycle:
            await act(.arrange, on: worktree) { c in await c.cycleLayout(worktree, layout: shown) }

        case .preset(let preset):
            await act(.arrange, on: worktree) { c in await c.applyPreset(preset, in: worktree, layout: shown) }

        case .evenPanes:
            // Which even arrangement, read off the panes rather than asked for.
            //
            // tmux has two — columns and rows — and picking the wrong one does
            // not "even out" a layout, it turns it inside out: a stack of three
            // becomes a row of three. So this counts how the window is already
            // split, the same way `TileView.Viewport` does, and hands back the
            // even version of the shape that is on screen.
            let columns = Set(group?.panes.map(\.left) ?? []).count
            let rows = Set(group?.panes.map(\.top) ?? []).count
            let preset: TilePreset = columns >= rows ? .evenHorizontal : .evenVertical
            await act(.arrange, on: worktree) { c in await c.applyPreset(preset, in: worktree, layout: shown) }

        case .splitRight, .splitDown:
            // One call. It used to be create-then-join-then-apply-a-preset, three
            // round trips whose only way of saying WHERE the new pane went was to
            // re-arrange every pane in the layout — so splitting the third pane of
            // four rebuilt the other three as well. `layout split` splits the pane
            // you name, on the side you name, and leaves the rest alone.
            let side: TileDirection = command == .splitRight ? .right : .bottom
            let groups = await act(.arrange, on: worktree, default: []) { c in
                await c.split(worktree, beside: here?.short, side: side, layout: shown)
            }
            // Land in the pane that was just made, which is the one tmux focuses.
            reveal(groups, in: worktree)

        case .breakPane:
            guard let here else { return }
            // Never the orchestrator: what shares its window moves out
            // instead, as Move to Its Own Window does.
            if let seat = WorkspaceScreen.seated(in: worktree, fleet: store.fleet).map(\.pane)
                .first(where: { $0.terminal.id == here.id })
            {
                let sharers = WorkspaceScreen.sharers(of: seat, layouts: store.client(for: worktree)?.layouts[worktree.id])
                if sharers.isEmpty { errorBanner = "The orchestrator already has a window of its own." }
                for sharer in sharers { await moveOutOfOrchestratorWindow(sharer, in: worktree) }
                return
            }
            let groups = await act(.arrange, on: worktree, default: []) { c in
                await c.breakPane(here.short, in: worktree)
            }
            reveal(groups, in: worktree, preferring: here.id)

        case .closePane:
            // tmux's `x`, and it means the same thing: the pane's process ends.
            // Routed through the existing close so there is one implementation of
            // what closing a terminal does.
            run(.closeTerminal)

        case .newGroup:
            await openTerminalInNewLayout(worktree)

        case .nextGroup:
            // Landing in the new layout is the point of switching to it. Without
            // this the selection stayed on a pane from the OLD layout, which the
            // detail view then showed on its own — so ⌃B n looked like it opened a
            // random terminal and came back.
            //
            // Through the layouts the bar offers. See `layout(stepping:from:in:)`.
            guard let next = Self.layout(stepping: 1, from: group?.id, in: screen?.groups ?? [])
            else { return }
            let groups = await act(.arrange, on: worktree, default: []) { c in
                await c.selectLayout(next.id, in: worktree)
            }
            reveal(groups, in: worktree)
        case .previousGroup:
            guard let previous = Self.layout(stepping: -1, from: group?.id, in: screen?.groups ?? [])
            else { return }
            let groups = await act(.arrange, on: worktree, default: []) { c in
                await c.selectLayout(previous.id, in: worktree)
            }
            reveal(groups, in: worktree)

        case .toggleAgentPane:
            // Falls back to the plain selection when there is no tmux group
            // yet — the few seconds `detail`'s own comment describes, before
            // the first `layout show` has come back, where `here` is nil but
            // a terminal is still very much selected.
            let target =
                here.flatMap { rect in worktree.terminals.first { $0.id == rect.id } }
                ?? selectedTerminal?.terminal
                // Selecting a LAYOUT TAB is not selecting a terminal, and a
                // cached layout does not always mark a pane focused — so both
                // of the above are nil for the commonest way of getting here,
                // and the command silently did nothing at all. A layout with
                // one pane has no ambiguity about which pane is meant.
                ?? group?.panes.first.flatMap { pane in
                    worktree.terminals.first { $0.id == pane.id }
                }
            guard let target else {
                // Never silent. A keystroke that does nothing and says nothing
                // is indistinguishable from a broken feature.
                errorBanner = "No pane to switch — select a terminal first."
                return
            }
            // Said here rather than left to the daemon's refusal, because this
            // is where the agent's NAME is known. The on-pane button is hidden
            // for a pane that cannot switch; the keystroke was not, so ⌃B a on
            // a Codex pane did nothing and explained nothing.
            guard target.canSwitchPaneMode || target.isAgentPane else {
                // Two different refusals, not one. A plain shell was never an
                // agent and telling it to add a config.toml entry is bad
                // advice; an agent Far Cooler recognizes but cannot host
                // needs exactly that entry. Mirrors the daemon's own two
                // refusal strings in `Service::set_pane_mode`.
                if target.hasDetectedAgent {
                    let agent = target.agentLabel
                    errorBanner =
                        "\(agent) has no chat adapter, so it stays a terminal. Add one in "
                        + "~/.config/farcooler/config.toml, then restart the daemon "
                        + "(farcooler daemon ensure) — it only reads the file at startup."
                } else {
                    errorBanner = "Nothing here to chat with — this pane isn’t running an agent."
                }
                return
            }
            await togglePaneMode(target, in: worktree)

        case .help:
            showShortcuts = true
        }
    }

    /// Ask the daemon to flip a pane between its terminal and its agent chat.
    ///
    /// One call, and the daemon is the one deciding whether that is even
    /// possible — a client guessing "this preset can't be an agent" would be
    /// exactly the kind of state the design says clients never derive.
    func togglePaneMode(_ terminal: Terminal, in worktree: Worktree) async {
        let target = terminal.isAgentPane ? "terminal" : "agent"
        let result = await act(
            .switchMode, on: worktree, target: terminal.id, subject: Self.quoted(terminal),
            default: DaemonClient.PaneModeResult.ok
        ) { c in
            await c.setPaneMode(terminal.short, mode: target)
        }
        switch result {
        case .ok, .failed:
            // A failure is already this action's result via `act`, so there is
            // nothing further to do from here.
            break
        case let .confirmationRequired(message):
            pendingPaneModeSwitch = PaneModeConfirmation(
                worktree: worktree, terminal: terminal.short, mode: target, message: message)
        }
    }

    /// Send a terminal to another layout, or to one of its own.
    ///
    /// `nil` is `break-pane`: a layout with just this in it. Naming a layout moves
    /// the pane against that layout's focused pane, because "which layout" is only
    /// half an instruction — tmux has to be told which pane and which edge, and the
    /// menu has no way to ask. Dragging is how you say the other half, and the drop
    /// indicator is why that is easier than answering a dialog about it.
    private func moveToLayout(_ terminal: Terminal, in worktree: Worktree, group: PaneGroup?) {
        Task {
            let groups: [PaneGroup]
            if let group, let onto = group.panes.first(where: \.focused) ?? group.panes.first {
                groups = await act(.arrange, on: worktree, default: []) { c in
                    await c.movePane(terminal.short, onto: onto.short, side: .right, in: worktree)
                }
            } else {
                groups = await act(.arrange, on: worktree, default: []) { c in
                    await c.breakPane(terminal.short, in: worktree)
                }
            }
            reveal(groups, in: worktree, preferring: terminal.id)
        }
    }

    /// Drop a terminal on an edge of a pane: it splits that pane on that edge.
    ///
    /// One write for every drag in the app — a pane onto a pane, from a tile
    /// or a layout's bar — because they all say the same
    /// thing: this terminal, against that one, on this side. It was three
    /// operations while a layout was an ordered list, and the list could only
    /// express "before" and "after", which is why dropping on the left half of a
    /// pane and on its right half used to do the same thing.
    ///
    /// Move a divider, in cells. Answers whether the request was taken.
    ///
    /// Serialized rather than queued. A drag produces one of these per cell
    /// crossed and each is a round trip, so a fast drag would stack up dozens of
    /// requests that land after the pointer has stopped and walk the divider past
    /// where it was let go.
    ///
    /// Refusing has to be VISIBLE to the caller, which is what the return value is
    /// for. The handle counts cells the layout has actually moved by, so a refusal
    /// leaves them owed and the next mouse event asks for them again. Without
    /// that the handle counted a dropped request as done and threw its cells away
    /// — losing most of them over a fast drag, so the divider followed the pointer
    /// at a fraction of its speed.
    @discardableResult
    func resizeDivider(
        _ terminal: String, side: TileDirection, cells: Int, in worktree: Worktree
    ) -> Bool {
        guard cells != 0, !resizingDivider else { return false }
        guard let pane = store.client(for: worktree)?.group(holding: terminal, in: worktree.id)?
            .pane(terminal)
        else {
            return false
        }
        resizingDivider = true
        Task {
            await act(.arrange, on: worktree) { c in
                await c.resizePane(pane.short, side: side, cells: cells, in: worktree)
            }
            resizingDivider = false
        }
        return true
    }

    /// Works across layouts too: the pane leaves whichever one it was in.
    func placePane(
        _ dragged: String, onto target: String, side: TileDirection, in worktree: Worktree
    ) {
        let shorts = [dragged, target].compactMap { id in
            worktree.terminals.first { $0.id == id }?.short
        }
        guard shorts.count == 2 else { return }
        // The orchestrator is one pane (ov-78): nothing joins its window,
        // and it never leaves it.
        let window = store.client(for: worktree)?.group(holding: target, in: worktree.id)?.terminals ?? [target]
        if WorkspaceScreen.joinsOrchestrator(dragged, window: window, in: worktree, fleet: store.fleet) {
            errorBanner = "The orchestrator keeps a window of its own, so nothing can be put beside it."
            return
        }
        Task {
            let groups = await act(.arrange, on: worktree, default: []) { c in
                await c.movePane(shorts[0], onto: shorts[1], side: side, in: worktree)
            }
            reveal(groups, in: worktree, preferring: dragged)
        }
    }

    /// Select the active layout's focused pane after a layout command.
    ///
    /// Every command that changes which group is on screen goes through this, so
    /// "the thing I am looking at" and "the thing the layout says is focused" cannot
    /// disagree. `preferring` is for the cases where the command was about a
    /// specific terminal and that terminal should win.
    func reveal(
        _ groups: [PaneGroup], in worktree: Worktree, preferring: String? = nil
    ) {
        let host = worktree.host ?? ""
        guard let active = groups.first(where: { $0.isActive }) ?? groups.first else {
            // No layouts left, which now means no terminals left. Fall back to
            // the worktree rather than to a pane that no longer exists.
            if Self.shows(worktree, selection) { selection = Self.opening(worktree, terminal: nil, in: store.fleet) }
            return
        }
        let target = preferring.flatMap { active.terminals.contains($0) ? $0 : nil }
            ?? active.focused
            ?? active.terminals.first
        guard let target else {
            if Self.shows(worktree, selection) { selection = Self.opening(worktree, terminal: nil, in: store.fleet) }
            return
        }
        focus(PaneRef(host: host, worktree: worktree.id, terminal: target))
    }

    /// Whether `selection` opens `worktree` whole: the one case where a
    /// layout command's answer can move the selection within it.
    nonisolated static func shows(_ worktree: Worktree, _ selection: Selection?) -> Bool {
        selected(in: worktree, by: selection) != nil
    }

    /// Put the keyboard in `pane`, which is on screen or about to be.
    ///
    /// Within the view on screen: a worktree opened whole selects the pane
    /// in it, so its navigator row lights and it's what the window reopens on;
    /// in the conversation or a task's column, the selection stays and only
    /// the key pane moves. A pane on no column of this view goes to where it
    /// lives, as `land(on:)` does.
    func focus(_ pane: PaneRef) {
        changesFocus = nil
        keyboardOnBoard = false
        let inConversation = shown.contains { $0.column == .conversation && $0.contains(pane) }
        guard let next = WorkspaceScreen.focusing(pane, selection: selection, shown: shown, fleet: store.fleet)
        else {
            land(on: pane)
            return
        }
        if next != selection { selection = next }
        keyPane = pane
    }

    /// ⌥⌘1, ⌥⌘2, ⌥⌘3 (ov-92): the orchestrator selected and given the
    /// keyboard; the navigator given the keyboard; the main area, whatever
    /// it shows, given the keyboard.
    func focusWorkspaceColumn(_ command: AppCommand) {
        // A loose worktree beside its repository's navigator takes them too.
        guard let current = selection, workspaceScene(current) != nil else { return }
        switch command {
        case .focusConversation:
            let step = WorkspaceNavigation.boardStep(.conversation, from: boardState)
            if step.selectsOrchestrator { selectOrchestrator(keyboard: step.keyboard) } else { apply(step) }
        case .focusBoard:
            // Asked for by name, so it comes back if it was put away.
            navigatorHidden = false
            apply(WorkspaceNavigation.boardStep(.board, from: boardState))
        case .focusTask:
            apply(WorkspaceNavigation.boardStep(.task, from: boardState))
        default:
            break
        }
    }
}
