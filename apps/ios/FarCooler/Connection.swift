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
        /// There WAS a connection, it went away, and one is being made again.
        ///
        /// A fourth phase rather than a flag on `connected`, because the two
        /// existing candidates are each wrong in a way that shows on screen.
        /// `connecting` means "there has never been a fleet" and renders a
        /// spinner in place of the whole screen — showing that to someone
        /// reading a terminal because their Wi-Fi blinked throws away the
        /// thing they were looking at. `failed` means "stopped, waiting for
        /// you", and this is not stopped.
        ///
        /// Views keep rendering the last known fleet through this. The Mac
        /// settled that rule already — an unreachable machine still
        /// contributes its last good rows, because that is what keeps your
        /// mental map of the fleet stable while a laptop sleeps — and the
        /// phone should not disagree with it about the same machine.
        case reconnecting(attempt: Int)
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

    /// Bumped every time a session is replaced by a new one.
    ///
    /// What a live terminal stream watches. A stream is a second SSH channel
    /// on the session that just died, and `TerminalSession` recovers from
    /// losing one by falling back to polling — correct, and slower than it
    /// needs to be once there is a link to stream over again. This is how it
    /// finds out there is.
    @Published private(set) var reconnectGeneration = 0

    // Not private: a pushed `TerminalView` talks to the same host through this
    // same core, rather than opening a second SSH session just to poll one
    // terminal's screen.
    let core = ClientCore()
    private var poller: Task<Void, Never>?

    /// The machine this connection is for, remembered so a reconnection has
    /// something to reconnect TO. Before this, the host appeared only as an
    /// argument to `start` and was gone the moment it returned.
    private var host: Host?

    /// The armed retry, or the attempt in flight. One slot, so a second
    /// request to reconnect replaces the first rather than running alongside
    /// it — the same rule the Mac's `DaemonClient.retryTask` follows.
    private var reconnectTask: Task<Void, Never>?

    /// Whether anyone is looking.
    ///
    /// A poll is an SSH round trip and a radio wake-up, and while the app is
    /// backgrounded nothing reads the answer. Worse: iOS suspends the process
    /// mid-flight, which is one plausible way the session dies in the first
    /// place. Android's client has had this gate since it was written; this
    /// one polled a phone in a pocket every three seconds.
    private var isActive = true

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

    deinit {
        poller?.cancel()
        reconnectTask?.cancel()
    }

    func start(host: Host) async {
        poller?.cancel()
        reconnectTask?.cancel()
        self.host = host
        attempt += 1
        let mine = attempt
        phase = .connecting

        // Claimed here rather than in `init`, so switching machines hands the
        // slot to the connection that is now on screen. The old one's closure
        // captures `self` weakly and no-ops once it is gone.
        Reachability.shared.onShouldRetry = { [weak self] in self?.reconnectNow() }

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

    // MARK: - Staying connected

    /// How long to wait before the next attempt, in seconds.
    ///
    /// The same schedule as the Mac's `DaemonClient.backoffSeconds`,
    /// deliberately: "how long until it comes back" should have one answer
    /// across the three apps. Doubling from two seconds to a thirty second
    /// ceiling, with jitter, because several machines recovering from one
    /// network event must not retry in lockstep.
    static func backoff(attempt: Int) -> Double {
        min(30, pow(2, Double(attempt))) * Double.random(in: 0.8...1.2)
    }

    /// How long a machine that answered SSH but not Far Cooler waits.
    ///
    /// Five minutes, matching the Mac. No amount of retrying installs a
    /// daemon, so the exponential schedule — which exists to survive a burst
    /// of transient failures quickly — is the wrong tool and would just be
    /// noise every thirty seconds forever. Not giving up either: installing it
    /// later should be noticed without relaunching the app.
    private static let slowRetrySeconds: Double = 300

    /// A call came back saying the link is gone.
    ///
    /// Detected in `refresh()` alone. Every other call site swallows its
    /// errors, and chasing all twenty of them would buy nothing: the poller
    /// runs every three seconds, so the drop is noticed within one poll of
    /// whichever call first hit it, from one place instead of twenty.
    private func linkDropped() {
        guard phase == .connected else { return }
        poller?.cancel()
        poller = nil
        scheduleReconnect(attempt: 1, after: Self.backoff(attempt: 1))
    }

    private func scheduleReconnect(attempt: Int, after seconds: Double) {
        phase = .reconnecting(attempt: attempt)
        reconnectTask?.cancel()
        // `[weak self]` rather than binding before the sleep: the whole point
        // of the wait is to do nothing, and a screen that goes away mid-wait
        // must be able to deallocate rather than be held until the timer
        // happens to fire.
        reconnectTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(seconds))
            guard !Task.isCancelled else { return }
            await self?.reconnect(attempt: attempt)
        }
    }

    /// Retry at once, whatever the backoff had planned.
    ///
    /// The escape hatch for everything a timer cannot know: you walked back
    /// into Wi-Fi range, you woke the machine, or you can simply see that this
    /// is stuck. Deliberately available from `connected` too — that is the
    /// case where the app believes it is fine and the person holding it knows
    /// better.
    func reconnectNow() {
        guard host != nil else { return }
        poller?.cancel()
        poller = nil
        reconnectTask?.cancel()
        phase = .reconnecting(attempt: 0)
        reconnectTask = Task { [weak self] in await self?.reconnect(attempt: 0) }
    }

    private func reconnect(attempt: Int) async {
        guard case .reconnecting = phase, let host else { return }
        guard let key = Identity.privateKey() else {
            phase = .failed("This device has no SSH key and one could not be generated.")
            return
        }

        do {
            _ = try await core.connect(config: host.config(privateKey: key))
        } catch {
            // A `start` or a second `reconnectNow` landed while this attempt
            // was crossing the network. Its answer is the current one; this
            // one must not reach up and overwrite it.
            guard case .reconnecting = phase else { return }
            retryOrGiveUp(on: error, attempt: attempt)
            return
        }

        guard case .reconnecting = phase else { return }
        phase = .connected
        // Before the reads below, so anything watching for a new link learns
        // about it in the same turn the link exists.
        reconnectGeneration += 1
        await refresh()
        // Re-read rather than trust what a previous session reported: a
        // machine that dropped and came back may have gained a repository, and
        // staying invisible to the pickers until relaunch is the failure the
        // Mac's `onReconnect` seeding exists to prevent.
        await loadRepositories()
        startPolling()
    }

    /// What to do about an attempt that failed: wait longer, wait much longer,
    /// or stop and say why.
    ///
    /// Stopping is not a dead end — the chip, the app becoming active and the
    /// network coming back all still reach `reconnectNow()`. It is the
    /// difference between a screen that explains what to fix and one that
    /// spins forever over something retrying will never fix.
    private func retryOrGiveUp(on error: Error, attempt: Int) {
        let next = classify(error)
        guard case .failed(let message) = next else {
            // The host key is unknown again, which is a question for a human
            // and not something to retry past.
            phase = next
            return
        }

        switch Failure(message: message) {
        case .keyRejected, .hostKeyChanged, .noIdentity, .keyNotTrusted:
            phase = next
        case .daemonMissing:
            // Kept at the same rung: `attempt` drives the fast schedule, means
            // nothing at this cadence, and letting it climb would leave a
            // later, genuinely transient failure starting at the ceiling.
            scheduleReconnect(attempt: attempt, after: Self.slowRetrySeconds)
        case .unreachable, .stopped, .other:
            scheduleReconnect(attempt: attempt + 1, after: Self.backoff(attempt: attempt + 1))
        }
    }

    /// The app came to the foreground, or left it.
    ///
    /// The single most common way this feature will be experienced: a phone in
    /// a pocket for two hours, iOS suspending the process, the sockets dying,
    /// and the first thing anyone sees on unlock being a stale screen.
    func setActive(_ active: Bool) {
        let wasActive = isActive
        isActive = active
        guard active, !wasActive else { return }

        switch phase {
        case .connected:
            // After two hours suspended, "connected" is a claim rather than a
            // fact. Testing it now beats waiting out a poll interval to find
            // out, and if it holds this is one round trip nobody notices.
            Task { await refresh() }
        case .reconnecting, .failed:
            reconnectNow()
        case .connecting, .needsApproval:
            // Already in flight, or waiting on a person. Neither is helped by
            // starting over.
            break
        }
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
        // The armed retry too. "Stop waiting" that leaves a backoff ticking
        // underneath would put the spinner back thirty seconds later, which is
        // the opposite of what was asked for.
        reconnectTask?.cancel()
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
            // A failed poll is not a disconnection — unless the core says it
            // is. That distinction did not exist before: this swallowed every
            // error, which is right about one poll and wrong about the
            // hundredth, and left the app with no path out of `connected` at
            // all. Either way the last known fleet stays on screen rather than
            // blanking the screen someone is reading.
            if let core = error as? ClientCore.CoreError, case .disconnected = core {
                linkDropped()
            }
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
        poller?.cancel()
        poller = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(3))
                await self?.refreshIfWatched()
            }
        }
    }

    /// A poll that a backgrounded app does not make. See `isActive`.
    private func refreshIfWatched() async {
        guard isActive else { return }
        await refresh()
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

    /// A second (or third) terminal in a worktree you already have open.
    /// Always a shell — matching macOS's own "New terminal", which has never
    /// offered a preset picker either.
    func createTerminal(workspace: Workspace) async {
        _ = try? await createTerminal(
            workspace: workspace.id,
            title: "Terminal \(workspace.terminals.count + 1)",
            preset: "shell")
        await refresh()
    }

    func hideWorkspace(_ workspace: Workspace) async {
        _ = try? await core.call("workspace.hide", ["workspace": workspace.id])
        await refresh()
    }

    func unhideWorkspace(_ workspace: Workspace) async {
        _ = try? await core.call("workspace.unhide", ["workspace": workspace.id])
        await refresh()
    }

    /// What asking to remove a worktree came back with — mirrors macOS's
    /// `DaemonClient.RemoveWorktreeResult` so both apps' UIs make the same
    /// three-way distinction.
    enum RemoveWorktreeResult {
        case ok
        case confirmationRequired
        case failed(String)
    }

    /// `confirm` must be the workspace's exact task name, unless the
    /// worktree is clean, in which case it may be empty.
    func removeWorktree(_ workspace: Workspace, confirm: String) async -> RemoveWorktreeResult {
        let data: Data
        do {
            data = try await core.call(
                "workspace.remove_worktree", ["workspace": workspace.id, "confirm": confirm])
        } catch {
            await refresh()
            return .failed(error.localizedDescription)
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
