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
        /// settled that rule already — an unreachable runner still
        /// contributes its last good rows, because that is what keeps your
        /// mental map of the fleet stable while a laptop sleeps — and the
        /// phone should not disagree with it about the same runner.
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
        /// Nothing answered: wrong address, host asleep, off the network.
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

    /// Whether `fleet` is an answer or merely the absence of one.
    ///
    /// Set by the first successful `refresh` and never cleared, on the same
    /// terms as `fleet` and `inbox` themselves: a link that dropped does not
    /// un-know what the runner last said, and the screens that render this hold
    /// their contents through a reconnect rather than emptying. See `refresh`.
    @Published private(set) var hasFleet = false
    @Published private(set) var repositories: [Repository] = []

    /// What each worktree has changed, by workspace id, or empty until the
    /// first read.
    ///
    /// The `+N -M` the Mac's sidebar has shown per row since the review surface
    /// landed — `DaemonClient.refreshChangesInbox`, which builds the same
    /// dictionary from the same rows — and the one part of that surface a phone
    /// never asked for. Nothing was missing: `changes.inbox` is routed at
    /// `crates/client/src/ffi.rs:1400`, answered at
    /// `crates/client/src/session.rs:968`, and `InboxRow` was already sitting
    /// in `Changes.swift` with no caller.
    ///
    /// Keyed rather than kept as the list the wire sends, because every reader
    /// arrives with one workspace in hand and wants that workspace's row: a
    /// fleet of twenty rows each scanning a twenty-entry array is quadratic
    /// work to answer a question a dictionary answers once.
    ///
    /// Held from the last successful read when a read fails, on the same terms
    /// as the fleet itself — see `refresh()`. A count that vanishes reads as
    /// "this worktree has no changes any more", which is a claim, and a call
    /// that failed is not entitled to make one.
    @Published private(set) var inbox: [String: InboxRow] = [:]

    /// Whether `changes.inbox` has ever answered, on any link this app has had.
    ///
    /// `inbox` alone cannot say. An empty dictionary is both "every worktree on
    /// this runner is reviewed" and "this runner's daemon predates
    /// `changes.inbox`, so the call is refused on every poll and always will
    /// be", and those are the two things the glance surfaces exist to tell
    /// apart: one is a review count of zero, the other is no review count at
    /// all. See `FleetSnapshot.needsReview`, which carries the same distinction
    /// out to a widget that has no connection to ask.
    ///
    /// Never reset. A read that succeeded once proves the runner can answer,
    /// and a single dropped packet afterwards is not evidence that it cannot —
    /// the same reasoning that makes `loadInbox` hold the last good rows rather
    /// than blanking them.
    @Published private(set) var inboxRead = false

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

    /// One review store per worktree, outliving the panes that show them. See
    /// `ChangesStores` for why they cannot live in the view.
    lazy var changesStores = ChangesStores(core: core)
    private var poller: Task<Void, Never>?

    /// The runner this connection is for, remembered so a reconnection has
    /// something to reconnect TO. Before this, the host appeared only as an
    /// argument to `start` and was gone the moment it returned.
    private var host: Runner?

    /// The runner this connection is for, as it reads on screen.
    ///
    /// `host` itself stays private — nothing outside should be able to change
    /// which runner a live connection points at — but a settings screen has to
    /// be able to title itself with the runner it is editing.
    var hostLabel: String { host?.label ?? "This runner" }

    /// The same runner, as everything that is not a screen spells one.
    ///
    /// A ceremony's records carry `Runner.id.uuidString`, and this connection
    /// can enroll a key on exactly one runner: the one it is connected to.
    /// Deciding WHICH granted runner that is has to be an id comparison —
    /// labels are chosen by people, two runners may share one, and enrolling a
    /// device on the wrong machine because two of them are called "mini" is not
    /// a mistake a person could see from either screen. Read-only for the same
    /// reason `hostLabel` is: what leaves here is a reading of the host, never
    /// a handle on it.
    var hostId: UUID? { host?.id }

    /// The connection the app is currently running, if it is running one.
    ///
    /// Weak, exactly as `WatchLinkHost` holds the same thing and for the same
    /// reason: `Connection` is a `@StateObject` owned by `FleetView`, switching
    /// runners replaces it, and a strong reference here would keep the old
    /// runner's SSH session alive forever — while still answering as though it
    /// were the runner the person is looking at.
    ///
    /// It exists because the enrollment ceremony has no other way to reach a
    /// session. `SettingsView` builds `AddDeviceView` from a `RunnerStore`
    /// alone; it holds a `Connection` and does not pass it on, so a
    /// `CeremonyStore` that took one as an argument would be a `CeremonyStore`
    /// nothing ever hands one to. See `CeremonyStore.throughTheLiveConnection`,
    /// which is the only reader.
    private(set) static weak var current: Connection?

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

    func start(host: Runner) async {
        poller?.cancel()
        reconnectTask?.cancel()
        self.host = host
        // The watch performs everything through whichever connection the app is
        // running, and this is the one place that knows which that is. Handed
        // over before connecting rather than after: `WatchLinkHost` checks the
        // phase itself, and registering only on success would answer "open the
        // app" to somebody who is holding it open while it reconnects.
        WatchLinkHost.shared.adopt(self)
        // The same handover, for the enrollment ceremony, and in the same
        // place so the two cannot disagree about which connection is current.
        // Also before connecting: `enroll` checks the phase itself, and the
        // ceremony is reached from a settings screen that a person can open
        // while this is still reconnecting.
        Connection.current = self
        attempt += 1
        let mine = attempt
        phase = .connecting

        // Claimed here rather than in `init`, so switching runners hands the
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
        await loadThemes()
        startPolling()
    }

    // MARK: - Staying connected

    /// How long to wait before the next attempt, in seconds.
    ///
    /// The same schedule as the Mac's `DaemonClient.backoffSeconds`,
    /// deliberately: "how long until it comes back" should have one answer
    /// across the three apps. Doubling from two seconds to a thirty second
    /// ceiling, with jitter, because several runners recovering from one
    /// network event must not retry in lockstep.
    static func backoff(attempt: Int) -> Double {
        min(30, pow(2, Double(attempt))) * Double.random(in: 0.8...1.2)
    }

    /// How long a host that answered SSH but not Far Cooler waits.
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
        // A `start` may still be waiting on the network — this is reachable
        // from `.connecting`, and a routable address with nothing listening
        // takes over a minute to say so. Bumping the counter is what stops its
        // answer landing on top of this one; see `attempt`.
        attempt += 1
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
        // runner that dropped and came back may have gained a repository, and
        // staying invisible to the pickers until relaunch is the failure the
        // Mac's `onReconnect` seeding exists to prevent.
        await loadRepositories()
        await loadThemes()
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
        if !active {
            // Give back the claim of attention on the way out, rather than
            // letting it age out on the runner. A pane still mounted behind a
            // locked phone has not been read by anybody, and an agent finishing
            // while the phone is in a pocket is precisely what the push exists
            // for — so the seconds between backgrounding and the claim expiring
            // are seconds this feature would spend swallowing the notification
            // it is least entitled to swallow. Best effort: iOS may suspend the
            // process before the call lands, which is exactly what
            // `WATCHED_TTL_MS` on the runner is the backstop for.
            //
            // Unconditional on `wasActive`, unlike the reconnect below: the
            // cost of one redundant release is a few bytes, and the cost of a
            // missed one is a notification nobody gets.
            Task { await reportWatching([]) }
        }
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
    func giveUp(on host: Runner) {
        abandon("Stopped waiting for \(host.address). It may be asleep or off the network.")
    }

    /// Back out of the fingerprint question without answering it.
    ///
    /// Lands on the failure screen rather than the spinner, because that is the
    /// screen with the runner switcher, the editor and this device's key on it.
    /// The wording is what `Failure.keyNotTrusted` matches on.
    func declineHostKey(_ host: Runner) {
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
            platform: body["platform"] as? String ?? "",
            // Absent from a daemon older than capabilities. `DaemonBuild.can`
            // reads an empty set as the features that existed then, so an old
            // runner keeps working rather than going dark.
            capabilities: Set(body["capabilities"] as? [String] ?? []))
        // Read from the same call, which is already made once per connection.
        //
        // Defaulted to the daemon's own default rather than to no prefix: an
        // older daemon that does not send the key still prefixes its branches
        // that way, so assuming nothing here would have this phone create
        // differently-named branches than the Mac beside it.
        branchPrefix = body["branchPrefix"] as? String ?? "feat/"
    }

    /// What this runner says a derived branch name starts with.
    ///
    /// Applied on this side rather than by the daemon, because the composer
    /// shows you the branch it is about to create.
    @Published private(set) var branchPrefix = "feat/"

    func refresh() async {
        guard phase == .connected else { return }
        do {
            let data = try await core.call("fleet")
            fleet = try JSONDecoder().decode(Fleet.self, from: data)
            // The runner has now told us what it has, at least once.
            //
            // `fleet` alone cannot answer this: `Fleet.empty` and a real fleet
            // from a runner with no worktrees are the same value, and the
            // difference between them is the difference between "we have not
            // asked" and "there is nothing". The inbox needs it — see
            // `FleetView.firstFleetArrived` — because "Nothing needs you" said
            // over a fleet nobody has read yet is the one sentence on that
            // screen that must never be transiently wrong, and `phase` cannot
            // stand in for it: `start` and `reconnect` both set `.connected`
            // and THEN await this call, so there is a whole round trip during
            // which the app is connected and knows nothing.
            hasFleet = true

            // The glance surfaces render from this and cannot fetch it
            // themselves. Written on every poll rather than on change, because
            // `capturedAt` is what makes the widget able to say how old it is,
            // and a snapshot only rewritten on change would claim to be as old
            // as the last state change rather than as old as the last look.
            //
            // The review count comes from the inbox read at the END of the
            // previous poll rather than from one taken here, and that is the
            // cheaper half of a deliberate trade. Reading it here would mean
            // moving `loadInbox` inside this `do`, where a throw is treated as
            // possible evidence the link is gone — see the note on the call
            // below, which is outside this block precisely so an old daemon
            // refusing the method cannot reconnect a working session every
            // three seconds. The cost is that the count is one poll, three
            // seconds, behind the agents beside it; the count is latched and
            // these surfaces are minutes to hours old by construction, so three
            // seconds buys nothing worth that.
            FleetSnapshotWriter.write(
                fleet: fleet, inbox: inboxRead ? inbox : nil, machine: hostLabel)

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

        // The diff counts, on the same poll as the rows they belong to.
        //
        // Here rather than on a timer of its own: `+120 -4` next to a row is a
        // statement about that row, and two clocks reading the same fleet is
        // how a count from a worktree's previous state ends up sitting beside a
        // fresh one. This is also the only loop a phone should be waking its
        // radio for — see `startPolling`.
        //
        // OUTSIDE the `do` above, deliberately. `refresh` treats a throw as
        // possible evidence the link is gone, and this call must never be able
        // to supply that evidence: an old daemon refusing `changes.inbox`
        // would otherwise reconnect a working session every three seconds.
        // `loadInbox` cannot throw at all, and a failed poll returns above
        // without reaching it, so a fleet nobody could read is never followed
        // by counts describing it.
        await loadInbox()
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
    ///
    /// It also renews this client's claim of ATTENTION, which is the same
    /// judgement one beat earlier — see `reportWatching`. Folded in here rather
    /// than given its own call sites because there is exactly one question
    /// underneath both ("is a person looking at this pane right now"), every
    /// place that already answers it calls this, and a second set of call sites
    /// is how the two answers come to disagree.
    func markVisibleSeen() async {
        guard phase == .connected else { return }
        guard UIApplication.shared.applicationState == .active else { return }
        await reportWatching(Notifier.shared.visibleTerminal.map { [$0] } ?? [])
        guard let id = Notifier.shared.visibleTerminal else { return }
        guard
            let terminal = fleet.workspaces.lazy.flatMap(\.terminals).first(where: { $0.id == id }),
            terminal.agent == .done
        else { return }
        guard markingSeen.insert(id).inserted else { return }
        defer { markingSeen.remove(id) }

        _ = try? await core.call("terminal.seen", ["terminal": id])
    }

    /// The terminals this phone last told the runner it was showing, and when.
    ///
    /// Both halves, because the send is skipped on two different grounds — see
    /// `reportWatching`.
    private var watching: [String] = []
    private var watchingSentAt: Date = .distantPast

    /// The shortest gap between two identical claims of attention.
    ///
    /// Two seconds, under the three-second poll that renews them, so the poll
    /// sets the real cadence and this only absorbs the extra calls a person
    /// generates by swiping through a pager. The runner believes a claim for ten
    /// seconds — `WATCHED_TTL_MS` in `crates/daemon/src/watch.rs` — so renewing
    /// on the poll leaves room for two lost round trips before a pane somebody
    /// is plainly looking at starts buzzing them again.
    private static let watchingFloor: TimeInterval = 2

    /// Tell the runner which pane is in front of the person, so it does not push
    /// a notification about one they are already reading.
    ///
    /// The complaint this exists for, in the owner's words: a codex pane open, a
    /// question asked, an answer given, and a buzz on the wrist about a reply
    /// already on screen. `markVisibleSeen` above cannot fix that, and this is
    /// the reason there are two calls rather than one — `terminal.seen` runs on
    /// the poll AFTER the agent finished, up to three seconds later, by which
    /// time the push has crossed the relay and the watch has already tapped. A
    /// claim of attention is made in advance and is therefore already true at
    /// the moment the turn ends.
    ///
    /// Suppression on the runner is per terminal and across every device,
    /// because it is one person: this phone saying it can see a pane is also
    /// what silences the watch on their wrist. That is the point, not a
    /// side effect.
    ///
    /// The WHOLE set on every call, replacing whatever this client said last,
    /// so backing out of a pane releases it in the same round trip rather than
    /// leaving it silent until the claim ages out. An empty array is therefore a
    /// real and important call — "I am looking at nothing" — and the reason this
    /// is not simply skipped when there is nothing to name.
    ///
    /// Sent again only when the set CHANGED or the floor has passed, which for a
    /// phone that stays on one pane means one small call per poll. Nothing is
    /// sent while backgrounded at all: `refresh` does not run there — see
    /// `refreshIfWatched` — and `setActive(false)` releases the claim on the way
    /// out.
    ///
    /// Errors swallowed, like `loadInbox` and for a stronger reason: this
    /// decorates nothing and blocks nothing, and a runner too old to know the
    /// method refuses it on every poll forever. Failing to be believed costs a
    /// notification somebody did not need; failing loudly would cost the fleet
    /// its screen.
    func reportWatching(_ terminals: [String]) async {
        guard phase == .connected else { return }
        // `"watching"` is `farcooler_protocol::capability::WATCHING`. A runner
        // that predates it withholds nothing and notifies exactly as it always
        // did, which is the honest fallback — but this is a HEARTBEAT, so
        // without the check it would spend a failing round trip every few
        // seconds, for as long as a pane is on screen, to be refused the same
        // way every time.
        guard daemon?.can("watching") ?? false else { return }
        let changed = terminals != watching
        guard changed || Date().timeIntervalSince(watchingSentAt) >= Self.watchingFloor else {
            return
        }
        // Recorded before the call rather than after, and recorded even if it
        // fails, for the reason `changesInboxReadAt` on the Mac is: a runner
        // that refuses this in a millisecond must not thereby be asked a
        // thousand times a second. The floor is a floor on ATTEMPTS.
        watching = terminals
        watchingSentAt = Date()
        _ = try? await core.call("terminal.watching", ["terminals": terminals])
    }

    private func loadRepositories() async {
        guard let data = try? await core.call("repositories") else { return }
        repositories = (try? JSONDecoder().decode(RepositoryList.self, from: data))?.repositories ?? []
        // Rebuilt here because this is the only place `repositories` is ever
        // written, so the two cannot drift apart. See `repositoryName(for:)`.
        repositoryNames = Dictionary(
            repositories.map { ($0.id, $0.displayName) }, uniquingKeysWith: { first, _ in first })
    }

    /// The display names, by repository id. Derived from `repositories`, which
    /// is a list because that is what the picker draws and what the wire sends.
    private var repositoryNames: [String: String] = [:]

    /// What to call the repository a worktree belongs to, or nil when this
    /// connection cannot say yet.
    ///
    /// `Workspace.repository` is a UUID string on a phone — `uuid_of(&w
    /// .repository_id).to_string()`, `crates/client/src/session.rs:274` —
    /// where the Mac's model carries a name it can print. So anything here that
    /// wanted to group or label rows by repository had 36 characters of hex and
    /// nothing else, while the names themselves were already in hand: they
    /// arrive on the `repositories` call this connection makes once per link,
    /// and this is the join between the two.
    ///
    /// Nil rather than the id when the name is not known, which is a real state
    /// and not an edge case: `loadRepositories` runs AFTER the first `refresh`
    /// in both `start` and `reconnect`, so the first fleet a person sees is
    /// drawn before any name exists — and that call swallows its own errors, so
    /// "not yet" and "not at all" look the same from here. A caller shows
    /// nothing in either case. Showing the UUID instead would put an internal
    /// identifier in front of somebody as though it were the name of their
    /// repository, and it would be the widest thing on the row.
    func repositoryName(for workspace: Workspace) -> String? {
        guard let id = workspace.repository else { return nil }
        return repositoryNames[id]
    }

    /// Read what every worktree on this runner has changed, in one call.
    ///
    /// One call rather than one per row, which is what the RPC exists for: the
    /// daemon answers from counts it already holds plus a cheap two-syscall
    /// gate — see `InboxWorkspace.changed_since_reviewed` in
    /// proto/farcooler.proto — so a quiet fleet costs almost nothing, while a
    /// call per worktree would put a `git` on the three-second timer for every
    /// row on screen. The Mac made the same choice for the same reason.
    ///
    /// Errors swallowed with `try?`, as `loadRepositories` and `loadThemes`
    /// beside it do, and with more reason than either: these numbers decorate
    /// rows that are already correct without them. A runner whose daemon
    /// predates `changes.inbox` fails this call on every poll forever, and
    /// neither that nor one dropped packet may cost the fleet its screen.
    func loadInbox() async {
        guard let data = try? await core.call("changes.inbox") else { return }
        guard let reply = try? JSONDecoder().decode(InboxResponse.self, from: data) else { return }
        // Set here and not before the call: what this records is that the
        // runner ANSWERED, which is the fact the surfaces need. A daemon too
        // old to know the method never reaches this line, so its review count
        // stays "not told" rather than becoming a confident zero.
        inboxRead = true
        inbox = Dictionary(
            reply.items.map { ($0.workspaceId, $0) }, uniquingKeysWith: { _, latest in latest })
    }

    /// Merge whatever this runner defines into the picker.
    ///
    /// Read on every connection and every reconnection, alongside
    /// repositories, so a `[themes.*]` table added to the host's config.toml
    /// does not stay invisible until the app is relaunched.
    private func loadThemes() async {
        guard let data = try? await core.call("themes") else { return }
        struct Reply: Decodable {
            var themes: [Theme]
        }
        guard let reply = try? JSONDecoder().decode(Reply.self, from: data) else { return }
        Themes.shared.merge(hostThemes: reply.themes)
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

    // MARK: - Runner settings
    //
    // Editing what the connected runner's config.toml holds. Every write
    // answers with the file's new state, read back by the daemon rather than
    // echoed from the request, so a value the writer normalized is what this
    // phone ends up holding.

    /// Only the themes this runner's file defines.
    ///
    /// Not the merged list `Themes.shared.available` holds: that one includes
    /// this phone's built-ins, and a built-in shown in an editor as if the file
    /// defined it would offer a delete that does nothing.
    func hostThemes() async -> [Theme] {
        guard let data = try? await core.call("themes"),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return [] }
        return Self.themes(from: body)
    }

    /// The branches in a repository, newest first as the daemon orders them.
    ///
    /// What makes "resume onto a branch" possible from a phone: work arrives on
    /// a branch at least as often as it starts on one — pushed from another
    /// machine, handed over, or produced by a cloud agent — and before this the
    /// only way to pick one up here was to type its name exactly.
    func branches(repository: String) async -> [Branch] {
        guard let data = try? await core.call("branch.list", ["repository": repository])
        else { return [] }
        return (try? JSONDecoder().decode(BranchList.self, from: data))?.branches ?? []
    }

    /// A branch's parent chain and the PR state along it.
    func stack(repository: String, branch: String) async -> StackResponse? {
        guard let data = try? await core.call(
            "stack.get", ["repository": repository, "branch": branch])
        else { return nil }
        return try? JSONDecoder().decode(StackResponse.self, from: data)
    }

    /// Ask GitHub again rather than answering from what was last read.
    func refreshPullRequests(repository: String) async -> StackResponse? {
        guard let data = try? await core.call("pr.refresh", ["repository": repository])
        else { return nil }
        return try? JSONDecoder().decode(StackResponse.self, from: data)
    }

    /// What the runner says about itself.
    ///
    /// The fleet already carries `runtime_healthy` as a bare bool, which is
    /// enough to tint a chip and not enough to act on: "something is wrong" is
    /// not an actionable sentence. This is the same `host.health` the Mac reads,
    /// with the daemon's own reasons attached.
    func health() async -> HostHealth? {
        guard let data = try? await core.call("host.health") else { return nil }
        return try? JSONDecoder().decode(HostHealth.self, from: data)
    }

    /// The repositories Far Cooler knows on this runner, and the roots it
    /// discovered them under.
    ///
    /// Two calls rather than one because they answer different questions — what
    /// you can start work in, and which directories the daemon is allowed to
    /// look in — and only the second is removable.
    func repositoryRoots() async -> [RepositoryRoot] {
        guard let data = try? await core.call("repository_root.list") else { return [] }
        return (try? JSONDecoder().decode(RepositoryRootList.self, from: data))?.roots ?? []
    }

    /// Stop watching a directory. The repositories already registered under it
    /// are the runner's business, not this call's.
    func removeRepositoryRoot(_ id: String) async -> Bool {
        ((try? await core.call("repository_root.remove", ["root": id])) != nil)
    }

    func setBranchPrefix(_ prefix: String) async -> String? {
        guard let data = try? await core.call(
            "settings.set_branch_prefix", ["prefix": prefix]),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        let stored = body["branchPrefix"] as? String ?? prefix
        branchPrefix = stored
        return stored
    }

    func upsertTheme(_ theme: Theme) async -> [Theme]? {
        guard let data = try? await core.call("theme.upsert", theme.arguments),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return Self.themes(from: body)
    }

    func deleteTheme(_ name: String) async -> [Theme]? {
        guard let data = try? await core.call("theme.delete", ["name": name]),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return Self.themes(from: body)
    }

    /// Put the runner's themes back into the picker every screen reads.
    ///
    /// Without this, a theme you just made is missing from the one place you
    /// would go to choose it.
    func reloadThemes() async {
        Themes.shared.merge(hostThemes: await hostThemes())
    }

    func adapters() async -> [AdapterInfo] {
        guard let data = try? await core.call("adapters"),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return [] }
        return Self.adapters(from: body)
    }

    func upsertAdapter(_ adapter: AdapterInfo) async -> [AdapterInfo]? {
        guard let data = try? await core.call("adapter.upsert", adapter.arguments),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return Self.adapters(from: body)
    }

    func deleteAdapter(_ preset: String) async -> [AdapterInfo]? {
        guard let data = try? await core.call("adapter.delete", ["preset": preset]),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return Self.adapters(from: body)
    }

    /// Prove an adapter works, without saving it first.
    func testAdapter(_ adapter: AdapterInfo) async -> AdapterTestOutcome {
        guard let data = try? await core.call("adapter.test", adapter.arguments),
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return .failed("That runner couldn’t be reached.") }
        if body["ok"] as? Bool == true {
            return .worked(body["reported"] as? String ?? "answered")
        }
        return .failed(body["failure"] as? String ?? "The adapter did not answer.")
    }

    private static func themes(from body: [String: Any]) -> [Theme] {
        (body["themes"] as? [[String: Any]] ?? []).compactMap(Theme.init(bridge:))
    }

    private static func adapters(from body: [String: Any]) -> [AdapterInfo] {
        (body["adapters"] as? [[String: Any]] ?? []).compactMap(AdapterInfo.init(json:))
    }

    /// `name` names the worktree's directory. The wire key is still `task`,
    /// which is what it was called when a workspace carried a typed-out task
    /// alongside its directory; renaming the key would strand every shipped
    /// app for nothing.
    /// `adopt` takes an EXISTING branch over instead of creating one. The
    /// worktree is then named after that branch, so `name` is ignored — the
    /// daemon says so too, and this passes it anyway rather than pretending the
    /// two calls have different shapes.
    func createWorkspace(
        repository: String, name: String, branch: String, adopt: Bool = false
    ) async {
        _ = try? await core.call(
            "workspace.create",
            // A shell, because a worktree with nothing running in it is a
            // directory. This is the manual form; the quick-task flow below
            // creates its own agent terminal and asks for none.
            [
                "repository": repository, "task": name, "branch": branch,
                "terminal": "shell", "adopt": adopt,
            ])
        await refresh()
    }

    // MARK: - Enrolling another device

    /// What became of one runner's half of a grant.
    ///
    /// Three outcomes rather than a Bool because two of them are different
    /// sentences: a runner whose Far Cooler predates device keys is fixed by
    /// updating it, and everything else is fixed by trying again from
    /// somewhere that can reach it. The Mac's `Enrollment` collapses its
    /// failures into one line on purpose — from the far side of an `ssh` a
    /// sleeping runner, an unloaded key and a damaged fence look identical —
    /// and this keeps the one distinction that IS knowable here, because the
    /// capability list arrived in the handshake rather than being guessed at.
    enum Enrollment: Equatable {
        /// The key is in that runner's `authorized_keys`.
        ///
        /// Answered at all is answered yes: `alreadyEnrolled` comes back in the
        /// daemon's result and is not a failure — it is the ordinary outcome of
        /// granting a runner the device can already reach — and the line is in
        /// the file either way, which is what was asked for.
        case written
        /// That runner's Far Cooler is older than device enrollment. Nothing
        /// was written, and no amount of retrying writes it.
        case tooOld
        /// Nothing was written, and the reason is not knowable from here: a
        /// runner that went away mid-ceremony, a daemon that refused because
        /// this device's own key is not enrolled with `host_admin` there
        /// (`crates/daemon/src/rpc.rs` requires it for `client.enroll`), and a
        /// fence that could not be rewritten all arrive as the same silence.
        case couldNotWrite
    }

    /// Put a device's key into this runner's `~/.ssh/authorized_keys`.
    ///
    /// **The daemon owns the write**, and that is the whole reason this is one
    /// call: that file is the one whose corruption costs somebody SSH access to
    /// their own machine, so the write is descriptor-anchored, `O_NOFOLLOW`,
    /// locked, atomic and `fsync`ed twice in `crates/daemon/src/enrollment.rs`.
    /// Nothing in this app appends a line to anything.
    ///
    /// `scope` is `control` per the design — `read` is a narrowing done
    /// afterwards in Settings › Devices — and there is deliberately no argument
    /// for the options, the forced command, or a `shellAccess` of true.
    /// `ClientEnroll` has no field for the first two, and the third is what
    /// asks for the PLAIN line Zed, git and Terminal need behind a shell; the
    /// FFI reads its absence as false, which is the restricted Key A line, and
    /// the absence of a way to ask for more from a phone is the guard rail. A
    /// phone granting to another phone is the `control` case, always.
    func enroll(publicKey: String, label: String, clientId: String) async -> Enrollment {
        guard phase == .connected else { return .couldNotWrite }

        // Asked BEFORE the call, the order `crates/cli/src/clients.rs` asks in
        // and for its reason: a runner too old to serve this answers "unknown
        // method", which is true and tells a person nothing they can act on —
        // and by then somebody has already scanned a code. The list arrived in
        // the handshake, so asking costs no round trip, and `loadDaemonBuild`
        // is a no-op once it has an answer.
        //
        // `"enrollment"` is `farcooler_protocol::capability::ENROLLMENT`, which
        // covers `client.list`, `client.enroll` and `client.revoke`. A daemon
        // that answered no capabilities at all predates the question entirely,
        // and `DaemonBuild.can` reads that as the two features that existed
        // then — so an ancient runner lands on `.tooOld` rather than on a call
        // it cannot serve.
        await loadDaemonBuild()
        guard let daemon else { return .couldNotWrite }
        guard daemon.can("enrollment") else { return .tooOld }

        // The keys are camelCase because that is what `crates/client/src/ffi.rs`
        // reads, and it checks the required three for PRESENCE by name rather
        // than defaulting them: a `public_key` sent here would come back as
        // "your key does not parse", which is a true statement about the empty
        // string and the wrong place to start looking.
        //
        // The client id is the caller's, and it is what the forced command will
        // carry — so it is what closing this device's sessions later will name.
        guard
            (try? await core.call(
                "client.enroll",
                [
                    "publicKey": publicKey,
                    "label": label,
                    "clientId": clientId,
                    "scope": "control",
                ])) != nil
        else { return .couldNotWrite }
        return .written
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
    func createWorkspace(repository: String, name: String, branch: String, base: String)
        async throws -> String
    {
        let data = try await core.call(
            "workspace.create",
            ["repository": repository, "task": name, "branch": branch, "base": base])
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

    /// `confirm` must be the workspace's exact name, unless the worktree is
    /// clean, in which case it may be empty.
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
