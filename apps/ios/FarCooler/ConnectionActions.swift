import Foundation

// What a person asks of one terminal or worktree that the runner may refuse:
// close, hide, unhide and switch a pane between its terminal and its chat.
// Each hands back an `ActionFailure` for the screen that asked to say (ov-179).
// Moved out of `Connection.swift`, which is past its size ceiling.

extension Connection {
    /// Close a terminal: stop whatever is in it, then delete the record.
    ///
    /// **Two calls because it has to be two calls.**
    /// `Service::remove_terminal` refuses a `Running` or `Starting` terminal
    /// with `RunningProcesses`, so the stop is not politeness — it is what
    /// makes the removal legal. And the removal is not optional: `remain-on-exit`
    /// keeps a dead pane deliberately, so a stop with no remove leaves a dead
    /// rectangle in the tab strip, which is a worse outcome than leaving the
    /// terminal running. Closing has to leave nothing behind, which is what
    /// closing means everywhere else. The Mac says the same thing in the same
    /// order at `ContentView.swift:2149-2155`.
    ///
    /// **Sequential, and it is safe to be.** `stop_terminal` kills the pane and
    /// then awaits `inventory.refresh()` before it answers, so by the time this
    /// makes the second call the state the second call inspects has already
    /// been re-derived. A parallel pair would be a race with the daemon's own
    /// refusal on the losing side.
    ///
    /// **One `refresh` at the end, not two.** `act` refreshes after each call
    /// and the first of those two is a fleet in which the pane is stopped but
    /// still listed — a dead rectangle, published to every screen, for one
    /// round trip. Going through `core` directly here keeps the intermediate
    /// state off the phone entirely: what a person sees is the pane, and then
    /// no pane.
    ///
    /// **A refused remove is said (ov-179).** It used to be swallowed, because
    /// what comes back is `RunningProcesses`, a Rust enum's name, and no raw
    /// runner error reaches a screen in this app. What the caller gets back now
    /// is `ActionFailure`, whose sentence is the refusal table's. The stop's own
    /// answer is dropped on purpose: closing is the remove that follows, so a
    /// stop that was refused is either followed by a remove that says why or
    /// by a tab that is gone.
    @discardableResult
    func close(terminal: Terminal) async -> ActionFailure? {
        _ = try? await core.call("terminal.stop", ["terminal": terminal.id])
        var failure: ActionFailure?
        do {
            _ = try await core.call("terminal.remove", ["terminal": terminal.id])
        } catch {
            failure = ActionFailure(
                error, title: "Couldn’t close the tab",
                otherwise: "That runner wouldn’t close it, so the tab is still there.")
        }
        await refresh()
        return failure
    }

    /// Put a worktree away, or take it back out.
    ///
    /// Fire-and-refresh: hiding is a view preference the runner stores, and
    /// the answer is the worktree moving into, or out of, its workspace's
    /// Hidden section. Nothing about where the worktree is changes.
    @discardableResult
    func hideWorktree(_ worktree: Worktree) async -> ActionFailure? {
        await moveWorktree("worktree.hide", worktree, title: "Couldn’t hide the worktree")
    }

    @discardableResult
    func unhideWorktree(_ worktree: Worktree) async -> ActionFailure? {
        await moveWorktree("worktree.unhide", worktree, title: "Couldn’t unhide the worktree")
    }

    private func moveWorktree(
        _ method: String, _ worktree: Worktree, title: String
    ) async -> ActionFailure? {
        var failure: ActionFailure?
        do {
            _ = try await rpc(method, ["worktree": worktree.id])
        } catch {
            failure = ActionFailure(
                error, title: title, otherwise: "That runner didn’t take it, so nothing moved.")
        }
        await refresh()
        return failure
    }

    /// Switch a pane between its terminal and its chat.
    ///
    /// Refreshes afterwards rather than guessing: the daemon respawns the pane,
    /// and what comes back — a new epoch, a different pane mode, possibly a
    /// refusal because a turn was in flight — is its answer to give, not this
    /// client's to assume.
    ///
    /// Returns the refusal, for the screen that asked (ov-179). The Mac offers
    /// to force a switch when a turn is in flight; the phone says it wouldn't.
    @discardableResult
    func setPaneMode(_ terminal: Terminal, to mode: String) async -> ActionFailure? {
        var failure: ActionFailure?
        do {
            _ = try await core.call(
                "terminal.set_pane_mode", ["terminal": terminal.id, "paneMode": mode])
        } catch {
            failure = ActionFailure(
                error, title: "Couldn’t switch the pane",
                otherwise: "That runner wouldn’t switch it just now. Try again in a moment.")
        }
        await refresh()
        return failure
    }

    /// Turn the runner's projector on or off (`settings.set_projector`,
    /// ov-373), then reconnect: a hello offers `agent_rows` only while it's
    /// on, and this link's hello was made before. Nil when it took; the
    /// sentence to show when it didn't.
    func setProjector(_ on: Bool) async -> String? {
        do {
            _ = try await rpc("settings.set_projector", ["on": on])
        } catch {
            return ClientCore.refusalWord(of: error) == RunnerRefusal.scopeDenied.rawValue
                ? "This device can’t change this runner’s settings."
                : "That runner didn’t take the change. Try again in a moment."
        }
        projectorOn = on
        reconnectNow()
        return nil
    }
}
