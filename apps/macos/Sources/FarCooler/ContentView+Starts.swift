import AgentKit
import AppKit
import SwiftUI

/// Starts: starting a task, resuming a branch, and opening a terminal or a shell.
///
/// A pure move out of `ContentView.swift`, which had outgrown its size ceiling;
/// nothing here changed when it moved.
extension ContentView {
    /// Start a task and go to it as soon as it exists.
    ///
    /// Selects the worktree and its agent's terminal the moment
    /// `DaemonClient.startTask` has made them, which is before the agent has
    /// booted: the description travels as the agent's launch argument, so
    /// nothing is left to wait for. The point is to be looking at the thing
    /// you asked for while it starts. (This comment said so from the start,
    /// while `startTask` in fact waited up to a minute for the agent to look
    /// idle before returning. It is true now.)
    ///
    /// `host` is handed in rather than re-derived from `project` here — see
    /// `QuickCreate.chosen`, which resolves both together from the same
    /// picker selection. A lookup repeated at this end, from `project`
    /// alone, is exactly the shape that goes silently wrong: `project` can
    /// name a repository that has since been removed, or one on a runner
    /// whose `repositories` has not been re-read since a reconnect, and a
    /// lookup that finds nothing has to be answered with a refusal, not a
    /// fallback to this Mac.
    func startTask(_ request: TaskRequest) async -> TaskSubmission.Outcome {
        let host = request.host
        if let client = store.clients[host], client.state == .notInstalled {
            return .failed("Far Cooler isn’t installed on this runner, so the agent wasn’t started.")
        }
        guard store.refusal(for: host) == nil, let client = store.clients[host] else {
            return .failed(
                "Can’t reach this runner right now, so the agent wasn’t started. Try again once it’s back.")
        }
        let outcome = await client.startTask(
            project: request.project,
            description: request.description,
            name: request.name,
            agent: request.preset.isEmpty ? Preferences.shared.defaultAgent : request.preset,
            reusing: request.worktree,
            // Claimed for the workspace the window is in, when the new
            // worktree is going into that workspace's repository, and
            // otherwise for that repository's Main.
            workspace: Self.claim(
                newWorktreeIn: request.project, on: host, from: selection, in: store.fleet),
            // After the start has returned and the panel has let go of the
            // draft, so it's said over the pane instead.
            undelivered: { sentence in errorBanner = sentence })
        switch outcome {
        case .started(let worktree, let terminal, let name):
            // By the ids the create calls returned, not by a later look at
            // the fleet — which, this soon, may not have the terminal yet.
            // A new worktree with no task yet is opened whole, under the
            // workspace it was claimed for.
            selection = arrival(host: host, worktree: worktree, terminal: terminal)
            keyPane = PaneRef(host: host, worktree: worktree, terminal: terminal)
            return .started(name: name)
        case .failed(let sentence, let made):
            if let made { reveal(made.id) }
            // After the selection change, which clears the banner. The panel
            // shows it when it is still open; this is for when it is not.
            if !showQuickCreate { errorBanner = sentence }
            // The worktree it made, so starting again goes on in it.
            return .failed(
                sentence,
                left: made.map {
                    TaskSubmission.Left(
                        host: host, project: request.project, worktree: $0.id, name: $0.name)
                })
        }
    }

    /// Pick up an existing branch and open an agent in it.
    ///
    /// Same landing as starting a task, because it is the same act from the
    /// user's side: there is now a worktree with an agent in it and you want to
    /// be looking at it. The only difference is where the code came from.
    ///
    /// Routed the same way `startTask` is: `host` comes from the resume
    /// sheet's own picker selection rather than a `project`-keyed lookup
    /// repeated here — see `startTask`'s own doc comment for why that lookup
    /// belongs where the selection was made, not downstream of it.
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
