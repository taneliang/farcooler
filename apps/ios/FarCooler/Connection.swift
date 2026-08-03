import Foundation
import UIKit

/// One host's session and the state a view renders from it.
///
/// The rule the whole product rests on holds here too: this never computes a
/// terminal's state. It asks, and shows what the daemon derived. A phone that
/// re-derived could disagree with the daemon and with the Mac about the same
/// terminal, which is exactly the confusion the design removed everywhere else.
@MainActor
final class Connection: ObservableObject {
    enum Phase: Equatable {
        case connecting
        /// First contact: the host's fingerprint, awaiting a human.
        case needsApproval(String)
        case failed(String)
        case connected
    }

    enum Action {
        case restart, stop, dismissLost
    }

    @Published private(set) var phase: Phase = .connecting
    @Published private(set) var fleet: Fleet = .empty
    @Published private(set) var repositories: [Repository] = []

    // Not private: a pushed `TerminalView` talks to the same host through this
    // same core, rather than opening a second SSH session just to poll one
    // terminal's screen.
    let core = ClientCore()
    private var poller: Task<Void, Never>?

    deinit { poller?.cancel() }

    func start(host: Host) async {
        poller?.cancel()
        phase = .connecting

        guard let key = Identity.privateKey() else {
            phase = .failed("This device has no SSH key and one could not be generated.")
            return
        }

        do {
            _ = try await core.connect(config: host.config(privateKey: key))
        } catch {
            phase = classify(error)
            return
        }

        phase = .connected
        await refresh()
        await loadRepositories()
        startPolling()
    }

    /// Turn the core's message into a phase a view can act on.
    ///
    /// The unknown-host case is not a failure — it is a question — and it has
    /// to be told apart from one, or the user is shown "try again" for
    /// something retrying will never fix.
    private func classify(_ error: Error) -> Phase {
        let message = error.localizedDescription
        if let fingerprint = fingerprint(in: message) {
            return .needsApproval(fingerprint)
        }
        return .failed(message)
    }

    private func fingerprint(in message: String) -> String? {
        guard message.contains("is unknown") else { return nil }
        return message
            .split(separator: " ")
            .first { $0.hasPrefix("SHA256:") }
            .map(String.init)
    }

    /// What the daemon on the other end is, asked once per connection.
    ///
    /// Cached because it cannot change while connected — a daemon that
    /// restarted is a connection that dropped — and because the settings screen
    /// should not cost a round trip every time it opens.
    @Published private(set) var daemon: DaemonBuild?

    func loadDaemonBuild() async {
        guard phase == .connected, daemon == nil else { return }
        guard let data = try? await core.call("host"),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return }
        daemon = DaemonBuild(
            version: body["daemonVersion"] as? String ?? "unknown",
            matches: body["buildsMatch"] as? Bool ?? true,
            platform: body["platform"] as? String ?? "")
    }

    func refresh() async {
        guard phase == .connected else { return }
        do {
            let data = try await core.call("fleet")
            fleet = try JSONDecoder().decode(Fleet.self, from: data)

            // Announce anything worth announcing, from the fleet we just read.
            //
            // Here rather than in a view: a notification about an agent must
            // not depend on which screen happens to be open, and this is the
            // one place that learns what every agent is doing.
            for workspace in fleet.workspaces {
                for terminal in workspace.terminals {
                    Notifier.shared.report(terminal: terminal, workspace: workspace.task)
                }
            }

            // And end `done` for whatever is on screen, from the same fleet.
            //
            // Here as well as on the taps in `TerminalView`, because an agent
            // finishing while you sit reading it is not a tap — it is a poll,
            // and this is the poll. Without it, the one case that needs no
            // interaction at all is the one case that never clears.
            await markVisibleSeen()
        } catch {
            // A failed poll is not a disconnection. Keep showing the last known
            // fleet rather than blanking the screen someone is reading.
            return
        }
    }

    // MARK: - Attention

    /// Terminals with a `terminal.seen` already in flight, so a poll landing
    /// while the last one is still crossing the SSH link does not send a second.
    private var markingSeen: Set<String> = []

    /// End `done` for the terminal on screen, if anyone is there to see it.
    ///
    /// `done` is finished-and-UNSEEN. The phone had no way to say it had seen
    /// anything — `terminal.seen` was never called from here at all — so an
    /// agent you opened on the phone, read, and backed out of stayed `done`
    /// forever: still orange in the list, still counted, and still the thing
    /// every later notification was about. The Mac cleared it for you or nothing
    /// did.
    ///
    /// Gated on the app being ACTIVE, which is the distinction the feature rests
    /// on. An agent finishing while the phone is in a pocket is exactly what the
    /// push notification is for, and a screen that happens to still be mounted
    /// behind a locked phone has not been read by anyone.
    ///
    /// Only `done`. `blocked` is an agent waiting on an ANSWER — looking at a
    /// question does not answer it — and the daemon would refuse to clear it
    /// anyway, so sending it would be a round trip spent to be told no.
    ///
    /// Nothing is applied locally afterwards. The next poll brings the daemon's
    /// answer, and this client does not compute a terminal's state; see the note
    /// on the type.
    func markVisibleSeen() async {
        guard phase == .connected else { return }
        guard UIApplication.shared.applicationState == .active else { return }
        guard let id = Notifier.shared.visibleTerminal else { return }
        guard
            let terminal = fleet.workspaces.lazy.flatMap(\.terminals).first(where: { $0.id == id }),
            terminal.agent == .done
        else { return }
        guard markingSeen.insert(id).inserted else { return }
        defer { markingSeen.remove(id) }

        _ = try? await core.call("terminal.seen", ["terminal": id])
    }

    private func loadRepositories() async {
        guard let data = try? await core.call("repositories") else { return }
        repositories = (try? JSONDecoder().decode(RepositoryList.self, from: data))?.repositories ?? []
    }

    /// Poll while the view is up.
    ///
    /// Three seconds, not sub-second: every poll is an SSH round trip, and this
    /// is a phone with a battery. The states that matter here change on the
    /// scale of an agent finishing a task, not a keystroke.
    private func startPolling() {
        poller = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(3))
                await self?.refresh()
            }
        }
    }

    func act(_ action: Action, on terminal: Terminal) async {
        let method: String
        switch action {
        case .restart: method = "terminal.restart"
        case .stop: method = "terminal.stop"
        case .dismissLost: method = "terminal.dismiss_lost"
        }
        _ = try? await core.call(method, ["terminal": terminal.id])
        await refresh()
    }

    func createWorkspace(repository: String, task: String, branch: String) async {
        _ = try? await core.call(
            "workspace.create",
            ["repository": repository, "task": task, "branch": branch])
        await refresh()
    }

    // MARK: - Quick Task
    //
    // Three thin, throwing calls rather than one method that does the whole
    // "describe it and an agent starts" flow, unlike the Mac's
    // `DaemonClient.startTask`. That one method can afford to be silent about
    // which step failed because it prints one banner; QuickTaskView has a
    // progress row to keep honest ("creating worktree" vs. "starting agent"),
    // and it cannot narrate steps it was never told about individually. The
    // fire-and-refresh `createWorkspace` above stays as it is, unchanged, for
    // the manual form next to it — that flow only needs the new row to show up.

    private struct IdentifiedReply: Decodable { var id: String }

    /// Create a workspace and hand back its id.
    ///
    /// Throws instead of swallowing the error, because the caller has nothing
    /// to create a terminal in if this fails and needs to say so rather than
    /// press on silently.
    func createWorkspace(repository: String, task: String, branch: String, base: String)
        async throws -> String
    {
        let data = try await core.call(
            "workspace.create",
            ["repository": repository, "task": task, "branch": branch, "base": base])
        return try JSONDecoder().decode(IdentifiedReply.self, from: data).id
    }

    /// Create a terminal and hand back its id, the same way.
    func createTerminal(workspace: String, title: String, preset: String) async throws -> String {
        let data = try await core.call(
            "terminal.create", ["workspace": workspace, "title": title, "preset": preset])
        return try JSONDecoder().decode(IdentifiedReply.self, from: data).id
    }

    /// Send exact bytes to a terminal. `hex` must already be lowercase hex —
    /// this does no encoding of its own, because the one caller that exists
    /// (QuickTaskView) needs the sentence and the carriage return sent as two
    /// separate calls, not two strings joined into one payload here.
    func writeRaw(terminal: String, hex: String) async throws {
        _ = try await core.call("terminal.write", ["terminal": terminal, "hex": hex])
    }

    /// The terminal a Quick Task just created, read back out of the fleet.
    ///
    /// `terminal.create`'s reply is only an id — the daemon's answer to
    /// `fleet` is the one place `activity` lives, which is what the waiting
    /// loop polls. Looked up rather than cached, because each poll needs the
    /// freshest copy.
    /// Switch a pane between its terminal and its chat.
    ///
    /// Refreshes afterwards rather than guessing: the daemon respawns the pane,
    /// and what comes back — a new epoch, a different pane mode, possibly a
    /// refusal because a turn was in flight — is its answer to give, not this
    /// client's to assume.
    func setPaneMode(_ terminal: Terminal, to mode: String) async {
        _ = try? await core.call(
            "terminal.set_pane_mode", ["terminal": terminal.id, "paneMode": mode])
        await refresh()
    }

    func terminal(_ id: String, in workspace: String) -> Terminal? {
        fleet.workspaces.first { $0.id == workspace }?.terminals.first { $0.id == id }
    }
}
