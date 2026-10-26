import Foundation

/// Quick Task's calls on a runner, and the worktree and repository calls
/// beside them, moved out of `Connection.swift` (ov-818) as they were.
///
/// Three thin, throwing calls rather than one method that does the whole
/// "describe it and an agent starts" flow, unlike the Mac's
/// `DaemonClient.startTask`. That one method can afford to be silent about
/// which step failed because it prints one banner; QuickTaskView has a
/// progress row to keep honest ("creating worktree" vs. "starting agent"),
/// and it cannot narrate steps it was never told about individually. The
/// fire-and-refresh `createWorktree` in `Connection.swift` stays as it is, for
/// the manual form next to it — that flow only needs the new row to show up.
extension Connection {

    private struct IdentifiedReply: Decodable { var id: String }

    /// Create a worktree and hand back its id.
    ///
    /// Throws instead of swallowing the error, because the caller has nothing
    /// to create a terminal in if this fails and needs to say so rather than
    /// press on silently.
    ///
    /// `workspace` is the workspace whose screen it was made from, which it's
    /// claimed for (ruling 8). Nil, as from anywhere else, is Main.
    func createWorktree(
        repository: String, name: String, branch: String, base: String, workspace: String? = nil
    ) async throws -> String {
        var args: [String: Any] = [
            "repository": repository, "task": name, "branch": branch, "base": base,
        ]
        if let workspace { args["workspace"] = workspace }
        let data = try await core.call("worktree.create", args)
        return try JSONDecoder().decode(IdentifiedReply.self, from: data).id
    }

    /// Create a terminal and hand back its id, the same way.
    func createTerminal(worktree: String, title: String, preset: String) async throws -> String {
        let data = try await core.call(
            "terminal.create", ["worktree": worktree, "title": title, "preset": preset])
        return try JSONDecoder().decode(IdentifiedReply.self, from: data).id
    }

    /// What asking for a new terminal came back with.
    ///
    /// Three cases rather than a thrown error, the same shape as
    /// `RemoveWorktreeResult` and for the same reason: the caller has to draw a
    /// SENTENCE, and a sentence needs to know which of these happened — not
    /// what a runner wrote. No message crosses this boundary at all, which is
    /// the point. `WatchLinkHost.reason` makes the argument at length: the
    /// core's string is the daemon's `Display` output, written for a terminal,
    /// and putting it on a screen is not a message, it is a leak.
    enum NewTerminalResult {
        /// The runner made one, and this is its id.
        case created(String)
        /// The link is gone, as opposed to the request being refused. Answered
        /// by the core rather than guessed from a message here — the
        /// distinction `ClientCore.CoreError.disconnected` exists to carry.
        case disconnected
        /// The runner answered, and the answer was no.
        case refused
    }

    /// A second (or third) terminal in a worktree you already have open.
    ///
    /// **Always a shell, and deliberately no preset picker.** It matches
    /// macOS's own New Terminal, which has never offered one, and it is the
    /// argument `farcooler terminal create --preset` makes for its own default:
    /// you open a terminal and type `claude` into it, and Far Cooler notices
    /// what is running rather than being told in advance. A picker here would
    /// ask for a decision before there is anything to decide it with, on the
    /// device with the least room to show one.
    ///
    /// Titled the way the Mac titles it — see
    /// `ContentView.openTerminalInNewLayout` — so a worktree opened on both
    /// does not end up numbering its tabs two different ways.
    ///
    /// **The refusal is not swallowed**, unlike the fire-and-refresh calls
    /// around it. `setPaneMode`'s own note calls that swallowing a known gap;
    /// here it would be worse than a gap, because when a create is refused
    /// there is no new tab afterwards — so a silent failure and a successful
    /// one that is simply slow look identical, and the only thing on screen
    /// says nothing happened either way.
    func createTerminal(in worktree: Worktree) async -> NewTerminalResult {
        do {
            let id = try await createTerminal(
                worktree: worktree.id,
                title: "Terminal \(worktree.terminals.count + 1)",
                preset: "shell")
            await refresh()
            return .created(id)
        } catch {
            // Refreshed on the way out as well as on the way through. A
            // refusal is still news about the fleet — and one likely cause of
            // it is this side holding a worktree the runner no longer has.
            await refresh()
            if let core = error as? ClientCore.CoreError, case .disconnected = core {
                return .disconnected
            }
            return .refused
        }
    }

    /// What asking to remove a worktree came back with — mirrors macOS's
    /// `DaemonClient.RemoveWorktreeResult` so both apps' UIs make the same
    /// three-way distinction.
    enum RemoveWorktreeResult {
        case ok
        case confirmationRequired
        /// The message, and the stable word the runner named it by. The word
        /// travels without a sentence because the sentence a screen falls back
        /// to is that screen's, not this type's.
        case failed(String, word: String?)
    }

    /// `confirm` must be the worktree's exact name, unless the worktree is
    /// clean, in which case it may be empty.
    func removeWorktree(_ worktree: Worktree, confirm: String) async -> RemoveWorktreeResult {
        let data: Data
        do {
            data = try await core.call(
                "worktree.remove", ["worktree": worktree.id, "confirm": confirm])
        } catch {
            await refresh()
            return .failed(error.localizedDescription, word: ClientCore.refusalWord(of: error))
        }
        struct Reply: Decodable {
            var ok: Bool?
            var confirmationRequired: Bool?
        }
        let reply = (try? JSONDecoder().decode(Reply.self, from: data)) ?? Reply()
        await refresh()
        if reply.confirmationRequired == true { return .confirmationRequired }
        return .ok
    }

    /// Allowlist the folder a repository lives in. Always a remote host's
    /// path from this app — a phone has no filesystem of its own worth
    /// pointing at.
    func addRepositoryRoot(path: String) async throws {
        _ = try await core.call("repository_root.add", ["path": path])
    }

    /// Register the repository itself, once its parent folder is
    /// allowlisted. Hands back the new repository's id, so the caller can
    /// select it immediately.
    func registerRepository(path: String) async throws -> String {
        let data = try await core.call("repository.register", ["path": path])
        return try JSONDecoder().decode(IdentifiedReply.self, from: data).id
    }
}
