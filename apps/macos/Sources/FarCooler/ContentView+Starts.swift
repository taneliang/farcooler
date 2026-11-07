import AgentKit
import AppKit
import SwiftUI

/// Starts: resuming a branch, and opening a terminal or a shell. Tasks are the
/// orchestrator's to start; nothing here does it (ov-951).
///
/// A pure move out of `ContentView.swift`, which had outgrown its size ceiling;
/// nothing here changed when it moved.
extension ContentView {
    /// Pick up an existing branch and open an agent in it.
    ///
    /// There is now a worktree with an agent in it and you want to be looking
    /// at it. `host` comes from the resume sheet's own picker selection rather
    /// than a `project`-keyed lookup repeated here: a repository removed since,
    /// or on a runner not yet re-read after a reconnect, would otherwise be
    /// answered with a fallback to this Mac instead of a refusal.
    func resume(branch: String, host: String, project: String, agent: String) {
        Task {
            let key = ActionKey(verb: .resumeBranch, host: host, target: "\(project) \(branch)")
            let subject = "“\(branch)”"
            if let why = store.refusalSentence(for: host) {
                outcomes.settle(key, failure: "\(ActionVerb.resumeBranch.lead(subject)) \(why)")
                return
            }
            guard let client = store.clients[host] else { return }
            let created = await outcomes.perform(key, subject: subject, on: client) { c in
                await c.adoptBranch(project: project, branch: branch, agent: agent)
            }
            reveal(created)
        }
    }

    /// Select a freshly created worktree, preferring its terminal.
    func reveal(_ worktree: String?) {
        guard let worktree else { return }
        let found = store.fleet.worktrees.first { $0.id == worktree }
        let host = found?.host ?? ""
        selection = arrival(host: host, worktree: worktree, terminal: found?.terminals.first?.id)
    }

    /// Where a worktree this window just made lands, with `terminal` in it:
    /// opened whole under the workspace the window is in when the fleet
    /// hasn't listed it yet, which, this soon after making it, is usual.
    private func arrival(host: String, worktree: String, terminal: String?) -> Selection {
        if let found = self.worktree(host: host, id: worktree) {
            return Self.opening(found, terminal: terminal, in: store.fleet)
        }
        if case .workspace(host, let current, _)? = selection {
            return .workspace(host: host, workspace: current, focus: .worktree(worktree, terminal: terminal))
        }
        return .looseWorktree(host: host, worktree: worktree, terminal: terminal)
    }

    /// Create a terminal and go straight to it.
    ///
    /// Selecting it afterwards matters: you made a terminal because you want to
    /// type in it, and leaving the selection where it was means a second click
    /// to get to the thing you just asked for.
    func newTerminal(in worktree: Worktree) {
        Task { await openTerminalInNewLayout(worktree) }
    }

    /// A new terminal, in a layout of its own.
    ///
    /// Every way of making a terminal in a worktree goes through this — the
    /// worktree menu's New Terminal, ⌘T, the palette's action, ⌃B c.
    ///
    /// One call now, where it used to be two. A terminal IS a tmux window and a
    /// window IS a layout, so creating one already produces the layout; the
    /// separate "make a group, then put it in the group" step was describing a
    /// distinction that no longer exists.
    ///
    /// tmux's `c` opens a window with a shell in it. So does this.
    @discardableResult
    func openTerminalInNewLayout(_ worktree: Worktree) async -> Terminal? {
        guard
            let created = await act(
                .newTerminal, on: worktree, default: nil as Terminal?,
                { c in
                    await c.createTerminal(
                        in: worktree,
                        preset: "shell",
                        title: "Terminal \(worktree.terminals.count + 1)")
                })
        else { return nil }
        focus(PaneRef(host: worktree.host ?? "", worktree: worktree.id, terminal: created.id))
        return created
    }

    /// ⌃B %, ⌃B " or ⌃B c with the keyboard in the Orchestrator column, and
    /// New Terminal in the navigator's Terminals section (ov-178): a shell
    /// in the main checkout, in a window of its own, opened beside the board
    /// in `workspace`, else the workspace on screen. Never a split of the
    /// orchestrator's window, which would make a checkout terminal only the
    /// column could draw (ov-78).
    func openShell(besideOrchestratorIn checkout: Worktree, workspace: String? = nil) async {
        let host = checkout.host ?? ""
        let id: String
        if let workspace {
            id = workspace
        } else if case .workspace(host, let current, _)? = selection {
            id = current
        } else {
            return
        }
        guard
            let created = await act(
                .newTerminal, on: checkout, default: nil as Terminal?,
                { c in
                    await c.createTerminal(
                        in: checkout, preset: "shell", title: "Terminal \(checkout.terminals.count + 1)")
                })
        else { return }
        trail = nil
        keyboardOnBoard = false
        selection = .workspace(host: host, workspace: id, focus: .worktree(checkout.id, terminal: created.id))
        keyPane = PaneRef(host: host, worktree: checkout.id, terminal: created.id)
    }
}
