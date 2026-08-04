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

    /// What a failure MEANS, as opposed to what it says.
    ///
    /// A failure screen that offers the same button for every failure is a
    /// failure screen that is wrong most of the time: "Try again" fixes a host
    /// that was asleep and fixes nothing at all about a key this device was
    /// never authorized with, or a host key that changed underneath us. Each of
    /// these has exactly one useful next move and they are not the same move.
    ///
    /// Read off the message rather than a typed error because the message is
    /// all that crosses the FFI boundary — the core hands back Rust's `Display`
    /// output as a string and there is no code to switch on. The substrings are
    /// the ones in `crates/client/src/ssh.rs` and `session.rs`; each is a
    /// distinctive phrase from the middle of its message rather than a prefix,
    /// so wrapping the error in more context does not stop it matching.
    enum Failure {
        /// The host answered but does not know this device's key.
        /// `SshError::AuthRejected` — fixed by authorizing, not by retrying.
        case keyRejected
        /// The key presented is not the one we pinned. `SshError::HostKeyChanged`.
        /// Retrying is guaranteed to fail, and offering it would suggest this is
        /// a glitch rather than a decision someone has to make.
        case hostKeyChanged
        /// Nothing answered: wrong address, machine asleep, off the network.
        /// `SshError::Connect` — the one case where retrying is the right move.
        case unreachable
        /// SSH worked; Far Cooler is not installed over there.
        /// `SessionError::DaemonMissing`.
        case daemonMissing
        /// This device has no usable key, so no host will ever accept it.
        case noIdentity
        /// The user was shown a fingerprint and did not say yes. Not a fault at
        /// all — a decision that has been deferred — and the way back is the
        /// same screen again, not a retry that pretends something broke.
        case keyNotTrusted
        /// The user stopped waiting. Also not a fault, and it must not be
        /// headlined as one.
        case stopped
        case other

        init(message: String) {
            if message.contains("rejected this key") { self = .keyRejected }
            else if message.contains("is not the one Far Cooler has recorded") {
                self = .hostKeyChanged
            } else if message.contains("cannot reach") { self = .unreachable }
            else if message.contains("did not answer") { self = .daemonMissing }
            else if message.contains("no SSH key") { self = .noIdentity }
            else if message.contains("has not been trusted") { self = .keyNotTrusted }
            else if message.contains("Stopped waiting") { self = .stopped }
            else { self = .other }
        }

        /// Whether "Try Again" belongs BELOW the primary action as a second
        /// option. False where retrying is already the primary action (it would
        /// then appear twice) and false where it cannot work at all.
        var worthRetryingAsAlternative: Bool {
            switch self {
            case .keyRejected: return true
            case .hostKeyChanged, .keyNotTrusted: return false
            case .unreachable, .daemonMissing, .noIdentity, .stopped, .other: return false
            }
        }
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

    /// Which connection attempt is current.
    ///
    /// `start` awaits the core, and that wait cannot be interrupted from here:
    /// the call is a ticket the core resolves whenever the network gets round
    /// to it, and a routable address with nothing listening takes as long as the
    /// OS's TCP timeout — over a minute — to say so. Giving up therefore cannot
    /// stop the work. What it can do is stop this object caring about the
    /// answer, which is what the counter is for: every write to `phase` is
    /// guarded on the attempt that produced it still being the current one, so
    /// an attempt someone abandoned two screens ago cannot reach up later and
    /// change what they are looking at now.
    private var attempt = 0

    deinit { poller?.cancel() }

    func start(host: Host) async {
        poller?.cancel()
        attempt += 1
        let mine = attempt
        phase = .connecting

        guard let key = Identity.privateKey() else {
            if mine == attempt {
                phase = .failed("This device has no SSH key and one could not be generated.")
            }
            return
        }

        do {
            _ = try await core.connect(config: host.config(privateKey: key))
        } catch {
            if mine == attempt { phase = classify(error) }
            return
        }

        guard mine == attempt else { return }
        phase = .connected
        await refresh()
        await loadRepositories()
        startPolling()
    }

    /// Stop waiting, and say so.
    ///
    /// Not a way to abort the SSH attempt — see `attempt` — but a way off the
    /// spinner, which is the thing that was actually missing. A connection to a
    /// machine that is asleep shows the same indefinite spinner as one that is
    /// about to succeed, and until this existed the only way out of that was to
    /// kill the app.
    func giveUp(on host: Host) {
        abandon("Stopped waiting for \(host.address). It may be asleep or off the network.")
    }

    /// Back out of the fingerprint question without answering it.
    ///
    /// Lands on the failure screen rather than the spinner, because that is the
    /// screen with the machine switcher, the editor and this device's key on it.
    /// The wording is what `Failure.keyNotTrusted` matches on.
    func declineHostKey(_ host: Host) {
        abandon(
            "The key \(host.address) presented has not been trusted on this device. "
                + "Far Cooler won’t connect until it is.")
    }

    private func abandon(_ message: String) {
        attempt += 1
        poller?.cancel()
        phase = .failed(message)
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
