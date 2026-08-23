import AgentKit
// `NSApp.isActive` and `didResignActiveNotification`, which are how this file
// answers "is anybody actually looking at this window" — see `reportWatching`.
import AppKit
// `AnyCancellable`, for the one notification this client subscribes to itself.
import Combine
import Foundation

/// Where a runner's connection stands.
///
/// `notInstalled` is separate from `unreachable` because it is not a failure
/// worth retrying at full speed. It is a runner that needs `host install`,
/// and retrying it every second forever produces noise instead of the one
/// sentence that would fix it. It still gets checked again — every few
/// minutes, see `DaemonClient.scheduleRetry()` — because "never" is the
/// opposite failure: installing it later would otherwise go unnoticed until
/// someone restarts the app.
enum HostState: Equatable {
    case connecting
    case connected
    case reconnecting(attempt: Int)
    case unreachable(reason: String)
    case notInstalled

    var isUsable: Bool { self == .connected }

    /// What to tell someone whose action was refused, or nil to let it proceed.
    ///
    /// Only `.unreachable` and `.notInstalled` refuse. Those are the two states
    /// where a command was already tried against this runner and failed, so
    /// trying another is asking the same question twice. `.connecting` and
    /// `.reconnecting` mean "we do not know yet", not "no" — refusing on those
    /// would turn an ordinary transient stream drop on an otherwise-reachable
    /// runner into a read-only window for up to a 30s backoff, when the
    /// command itself would simply have succeeded, or failed on its own and
    /// bounded by `ConnectTimeout`, exactly as it would on a runner this
    /// client had never seen go down at all. A false refusal here is worse
    /// than the bounded wait a real attempt risks.
    var refusal: String? {
        switch self {
        case .connected, .connecting, .reconnecting: return nil
        case .unreachable(let why): return why
        case .notInstalled: return "Far Cooler is not installed on this runner"
        }
    }
}

/// Talks to the daemon through the `farcooler` CLI.
///
/// DEVIATION FROM THE DESIGN, recorded honestly: the accepted architecture has
/// the Mac app speak the protobuf protocol over the daemon's Unix socket. Night
/// one drives the CLI as a subprocess instead, because the socket transport was
/// still being built. The boundary is the same either way: the app never calls
/// git, SQLite, or tmux itself, and it never derives state. Swapping this one
/// file for a socket client is the whole migration.
@MainActor
final class DaemonClient: ObservableObject {
    /// Diff status per worktree short id, for the sidebar. Empty until read.
    @Published var changesInbox: [String: InboxRow] = [:]
    /// Whether this runner's daemon knows about review at all.
    ///
    /// `nil` until asked. `false` means the daemon ANSWERED and refused — which
    /// on a runner installed before review is every review call, because an
    /// unknown method is a `NOT_FOUND` there. Distinguished from "unreachable",
    /// which is not the daemon's answer and must not be remembered as one.
    @Published var changesSupported: Bool?
    /// The last thing review failed with, verbatim from the daemon.
    @Published var changesError: String?

    @Published var fleet: Fleet = .empty
    @Published var lastError: String?
    @Published var busy = false

    /// The runner this client drives. Empty means the Mac it runs on.
    ///
    /// An instance property, not a global preference. One client per runner is
    /// what lets an unreachable one be a single object in a bad state rather
    /// than a condition threaded through shared code — and it is what makes
    /// holding several at once possible at all.
    let target: String

    init(target: String = "") {
        self.target = target.trimmingCharacters(in: .whitespaces)
        watchResignations()
    }

    /// Cancels the retry loop rather than leaving it to run against a client
    /// nobody holds anymore.
    ///
    /// Without this, a removed host's `retryTask` keeps itself alive:
    /// `scheduleRetry`'s `Task` calls `startEvents`, whose `onEnd` arms the
    /// next `scheduleRetry` — self → retryTask → self, with no owner left to
    /// break the cycle. It would keep spawning `farcooler … events` and
    /// `workspace list` subprocesses against a runner this app no longer
    /// shows anywhere, invisible except in `ps`.
    deinit {
        retryTask?.cancel()
        // For the same reason, one clock over: `reportWatching`'s renewal
        // re-arms itself, so a client nobody holds anymore would go on
        // spawning a `farcooler terminal watching` every three seconds against
        // a runner this app no longer shows.
        watchingTask?.cancel()
    }

    @Published private(set) var state: HostState = .connecting

    /// Bumped every time this runner's link is replaced by a new one.
    ///
    /// What the per-pane streams watch. `state` alone cannot serve: it settles
    /// back to `.connected` and a view comparing it against its last value sees
    /// nothing to do, while a counter says "the link you started on is not the
    /// link that exists now" — which is the only question a stream needs
    /// answered. Same shape as the phones' `Connection.reconnectGeneration`,
    /// for the same reason and with the same name.
    @Published private(set) var linkGeneration = 0

    /// Which build is answering for this runner, or nil until this link's own
    /// read has come back.
    ///
    /// Per LINK, not per client: a daemon that was replaced is a different
    /// program on the same socket, so the answer is cleared the moment the
    /// link is, rather than being carried across a reconnection where it would
    /// describe something that is no longer there. That matters in the one
    /// direction that would be embarrassing — a runner someone has just
    /// updated by hand would otherwise go on being reported as behind until
    /// the app was relaunched.
    @Published private(set) var daemonBuild: DaemonBuild?

    /// Set when the read above ran and could not answer.
    ///
    /// Distinguished from `daemonBuild == nil`, which is only "nobody has
    /// asked yet". A runner that answered `workspace list` and then failed to
    /// answer `status` is a real case — `status` asks for host facts, and a
    /// daemon old enough to predate that method answers `NOT_FOUND` — and the
    /// two must not look the same, or the runner most likely to be stale
    /// would be shown as one whose read simply had not landed.
    @Published private(set) var daemonBuildUnreadable = false

    /// Whether what is running there is what this app was built to drive.
    ///
    /// Derived rather than stored, and derived from `state` FIRST, because
    /// connection health outranks version news in every case: a runner nobody
    /// can reach is not a runner with a stale daemon, whatever the last link
    /// happened to report about it. See `DaemonSkew`, which is where the
    /// reasoning and the copy live.
    var daemonSkew: DaemonSkew {
        Self.skew(
            state: state,
            build: daemonBuild,
            unreadable: daemonBuildUnreadable,
            remote: !target.isEmpty)
    }

    /// The rule itself, as a pure function of the four things it reads.
    ///
    /// Static and free of `self` so it can be tested. It decides whether
    /// somebody is told their runner is out of date, and both ways of getting
    /// it wrong are bad in a way that is invisible by looking: too eager and
    /// the sidebar nags about a runner that is fine, too shy and a feature
    /// silently does nothing — which is the bug this whole file exists to
    /// answer, and it went unnoticed for fourteen commits.
    ///
    /// `remote` rather than the target string, because the only thing the rule
    /// needs from it is whether "update the older side" could be about a
    /// runner at all. This Mac cannot be a protocol version behind itself.
    /// `nonisolated` because it touches nothing on the actor — every input is
    /// a parameter. That is what lets a test call it without a client.
    nonisolated static func skew(
        state: HostState, build: DaemonBuild?, unreadable: Bool, remote: Bool
    ) -> DaemonSkew {
        switch state {
        case .connecting, .reconnecting, .notInstalled:
            return .unavailable
        case .unreachable(let reason):
            // The one flavor of unreachable that IS a version. `explain` in
            // `crates/cli/src/remote.rs` says "update the older side" for a
            // refused handshake, and this client already gives that failure
            // the slow retry cadence because retrying cannot fix it.
            let mismatch =
                remote && reason.localizedCaseInsensitiveContains("update the older side")
            return mismatch ? .tooOldToTalk : .unavailable
        case .connected:
            guard let build else {
                return unreadable ? .unknown : .unavailable
            }
            return build.matches ? .current : .behind(daemon: build.readable)
        }
    }

    /// Called the moment `refresh()` transitions `state` into `.connected`
    /// from anything else — set by `FleetStore`, which uses it to re-seed
    /// repositories, roots and layouts on every reconnection, not only at
    /// bring-up. Not fired for a `refresh()` that merely confirms an
    /// already-`.connected` client is still up.
    var onReconnect: (() -> Void)?

    /// How long to wait before the next attempt, in seconds.
    ///
    /// Doubling from 1 to a 30s ceiling, with jitter: several runners
    /// recovering from one network event must not retry in lockstep, or the
    /// first thing a just-returned network sees is a thundering herd.
    private var attempt = 0
    private var retryTask: Task<Void, Never>?

    /// True from the moment `stopEvents()` runs until `startEvents()` or
    /// `reconnectNow()` next runs — checked by `scheduleRetry()` in addition
    /// to its `retryTask == nil` guard.
    ///
    /// `retryTask == nil` alone is not "not stopped"; it is "idempotent",
    /// and `stopEvents()` nils that same slot as part of shutting down. A
    /// `refresh()` already in flight when `stopEvents()` runs keeps running
    /// to completion — `runRaw` is not cancellation-aware — and its failure
    /// path calls `scheduleRetry()` directly, with no cancellation check of
    /// its own between it and the `Task` that was told to stop. Reading
    /// `retryTask == nil` at that moment, `scheduleRetry()` cannot tell "a
    /// retry has never needed to arm" from "this client was just told to be
    /// quiet", and arms a fresh one either way — resurrecting the exact loop
    /// `stopEvents()` exists to end. This flag is the fact `retryTask` alone
    /// cannot carry, checked wherever a retry would arm, so being stopped
    /// wins regardless of which of the two racing paths gets there first.
    private var isStopped = false

    private var backoffSeconds: Double {
        let base = min(30.0, pow(2.0, Double(attempt)))
        let jitter = Double.random(in: 0.8...1.2)
        return base * jitter
    }

    /// Whether a failure names a runner that needs installing rather than
    /// one that is merely unreachable right now.
    ///
    /// `crates/cli/src/remote.rs`'s `explain` produces this exact phrase when
    /// ssh connects fine but nothing answers `farcoolerd --stdio` on the far
    /// side, which from here is indistinguishable from "not installed" — and
    /// is the CLI's own signal for it, not a guess. Local-only: the Mac
    /// bundles and starts its own daemon, so a local failure is never "go
    /// install this".
    private func looksNotInstalled(_ message: String) -> Bool {
        !target.isEmpty && message.localizedCaseInsensitiveContains("is far cooler installed")
    }

    /// Whether a failure names two sides speaking different protocol
    /// versions.
    ///
    /// Same source as `looksNotInstalled`: `explain`'s other branch, for
    /// `ClientError::VersionMismatch`. No amount of retrying updates the
    /// older side, so this gets the same slow cadence as `notInstalled`
    /// rather than the fast exponential one — see `scheduleRetry()`.
    private func looksVersionMismatch(_ message: String) -> Bool {
        !target.isEmpty && message.localizedCaseInsensitiveContains("update the older side")
    }

    /// How long a runner that needs installing, or that speaks a different
    /// protocol version, waits between checks.
    ///
    /// Five minutes: no amount of retrying fixes either condition, so the
    /// exponential schedule (which only exists to survive a burst of
    /// transient failures quickly) is the wrong tool here and would just be
    /// noise every 30 seconds forever. This is the opposite failure — never
    /// checking again — which would mean installing it later, or updating
    /// the older side, goes unnoticed until someone restarts the app.
    private let slowRetrySeconds: Double = 300

    /// The one place that arms a retry, so `onEnd` and `refresh()`'s own
    /// failure paths cannot each schedule their own and race.
    ///
    /// Idempotent: if a retry is already armed and ticking down, this leaves
    /// it alone rather than cancelling and replacing it. Without that, an
    /// ordinary UI-driven `refresh()` call made while a host happens to be
    /// down — and `refresh()` has some thirty call sites — would push an
    /// already-scheduled retry further out every time someone clicked
    /// anything, and two near-simultaneous failures (`refresh()`'s own and
    /// `onEnd`'s, moments apart for the same underlying drop) would each
    /// bump `attempt` and skip a rung instead of counting as one failure.
    /// `reconnectNow()` is the deliberate exception to all of this: it
    /// forcibly cancels and replaces, because "now" means now.
    ///
    /// `refresh()` needs this as much as `onEnd` does: a `farcooler events`
    /// subprocess that hangs against a dead host rather than exiting —
    /// plausible now that ssh's keepalives (see `crates/cli/src/remote.rs`)
    /// hold the connection open for up to 45s — reports no `onEnd` at all,
    /// and without this a client in that state sits at `.unreachable`
    /// forever with nothing counting down.
    private func scheduleRetry() {
        guard !isStopped, retryTask == nil else { return }

        let wait: Double
        switch state {
        case .notInstalled:
            // `attempt` resets rather than climbs here: it drives the fast
            // exponential schedule only, means nothing at this cadence, and
            // letting it grow would leave a later, genuinely transient
            // failure starting at the 30s ceiling instead of a fresh 1s.
            attempt = 0
            wait = slowRetrySeconds
        case .unreachable(let reason) where looksVersionMismatch(reason):
            attempt = 0
            wait = slowRetrySeconds
        default:
            attempt += 1
            wait = backoffSeconds
        }

        // `[weak self]` alone is not enough here: binding `guard let self`
        // before the sleep would hold a strong reference for the entire
        // wait, for nothing — the whole point of the wait is to do nothing.
        // Deleting a client mid-backoff must let it deallocate, not keep it
        // alive until the timer happens to fire.
        retryTask = Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(wait))
            guard !Task.isCancelled, let self else { return }
            // A daemon going away is also the moment another build could
            // take the socket, so the app claims it back before reading
            // anything through it.
            if self.target.isEmpty { await LocalDaemon.shared.ensure() }
            guard !Task.isCancelled else { return }
            // Anything that changed while we were deaf is only visible in a
            // full read. This may itself call `scheduleRetry()` again on
            // failure — a no-op, since this task is still the recorded
            // `retryTask` at that point, which is exactly the idempotency
            // above. That is not a dead end: `startEvents()` below still
            // runs, and the subprocess it starts either survives (this
            // runner is back) or dies and fires `onEnd`, which finds
            // `retryTask` nil by then and arms the next wait correctly.
            await self.refresh()
            guard !Task.isCancelled else { return }
            self.retryTask = nil
            self.startEvents()
        }
    }

    /// Retry this runner at once, whatever the backoff had planned.
    ///
    /// The escape hatch for the case the timer cannot know about: you fixed the
    /// VPN, and waiting out a thirty second ceiling to find out is the wrong
    /// experience.
    ///
    /// Shares `retryTask`'s slot with `scheduleRetry()`: a second call, or one
    /// landing while a scheduled retry is already in flight, must cancel the
    /// other rather than run alongside it — one slot is what makes "cancel
    /// the previous one" the whole rule.
    func reconnectNow() {
        attempt = 0
        // Cancels and nils `retryTask` as a side effect — see `stopEvents()`
        // — which is what leaves the slot correctly empty for the
        // reassignment below rather than merely cancelled.
        stopEvents()
        // `stopEvents()` just set `isStopped`, which is right for a caller
        // that meant "be quiet" but wrong here — "now" means resume at once,
        // not stay stopped. Without this, the Task below's own `refresh()`
        // failing would call `scheduleRetry()` while `isStopped` is still
        // true and silently no-op, leaving `state` reading `.connecting`
        // forever with nothing counting down.
        isStopped = false
        state = .connecting
        retryTask = Task { @MainActor [weak self] in
            guard let self else { return }
            if self.target.isEmpty { await LocalDaemon.shared.ensure() }
            guard !Task.isCancelled else { return }
            await self.refresh()
            guard !Task.isCancelled else { return }
            self.retryTask = nil
            self.startEvents()
        }
    }

    /// Where the CLI lives.
    ///
    /// The bundled copy comes first, because an app launched from the Dock
    /// inherits no shell environment and cannot find something that only exists
    /// on your PATH. The env override and PATH lookups are for running from a
    /// checkout during development.
    var cliPath: String? { binary }

    var cliEnvironment: [String: String] { environment }

    /// What to put in front of every CLI invocation to aim it at this client's
    /// runner. Empty when it is the runner the app runs on.
    ///
    /// Before the subcommand, not after: `--host` is a top-level option, and
    /// clap will not see it once a subcommand has been named.
    var cliHostArguments: [String] {
        target.isEmpty ? [] : ["--host", target]
    }

    private var binary: String? { CLI.binary }

    private var environment: [String: String] { CLI.environment }

    // MARK: - Live updates

    private var eventStream: EventStream?
    /// Which stream is current, so a terminated one's `onEnd` can tell
    /// whether it still is.
    ///
    /// `EventStream.stop()` nils its caller's reference but cannot stop the
    /// child process's termination handler from still running — Process
    /// fires it for a killed child same as a crashed one. Without this, a
    /// deliberate `stopEvents()` (from `reconnectNow()`, or the window's
    /// `.onDisappear`) is followed milliseconds later by that same stream's
    /// `onEnd`, which would arm a whole new retry loop for a client nothing
    /// asked to keep running. Comparing generations rather than the
    /// `EventStream` instance itself sidesteps the same problem the other
    /// way: it also covers the stream that already replaced it, so `onEnd`
    /// landing late can never null out a healthy successor by mistake.
    private var streamGeneration = 0
    /// Terminals whose clean exit we have already acted on, so a burst of
    /// events for the same one does not queue several removals.
    private var reaped: Set<String> = []

    /// Start receiving pushed changes.
    ///
    /// Replaces polling. A poll has to choose between noticing an agent's
    /// question late and burning cycles on a fleet where nothing is happening;
    /// pushed changes have neither problem, and a quiet host sends nothing.
    func startEvents() {
        // Whatever called this wants the client live — the initial bring-up,
        // a scheduled retry that just succeeded, `reconnectNow()`'s own
        // task, or `FleetStore.resume()` bringing a closed window's fleet
        // back. Reset here, not just in `reconnectNow()`, so `scheduleRetry()`
        // is armed correctly regardless of which of those paths is calling.
        isStopped = false
        guard eventStream == nil else { return }
        guard let binary else {
            // No CLI to run means no stream can ever start on its own, so
            // without arming a retry here this state never has a way out.
            state = .unreachable(reason: "The farcooler CLI was not found.")
            scheduleRetry()
            return
        }

        streamGeneration += 1
        let generation = streamGeneration
        let stream = EventStream(
            onEvent: { [weak self] event in
                Task { @MainActor in
                    // Stale, same test `onEnd` uses and for the same reason:
                    // a line already in flight through the subprocess's pipe
                    // when `stopEvents()` runs is read and decoded regardless
                    // — `EventStream.stop()` tears down the process but
                    // cannot un-read bytes already sitting in the pipe — so
                    // without this a buffered event for a removed runner
                    // still mutates `fleet` and fires a notification for it.
                    guard let self, self.streamGeneration == generation else { return }
                    self.apply(event)
                }
            },
            onLayout: { [weak self] event in
                Task { @MainActor in
                    guard let self, self.streamGeneration == generation else { return }
                    self.layouts[event.workspace] = event.groups
                }
            },
            onFleet: { [weak self] in
                Task { @MainActor in
                    // Stale-guarded for the same reason as `onEvent` above,
                    // and doubly so here: an unguarded `refresh()` can itself
                    // call `scheduleRetry()` on failure, which is exactly the
                    // resurrection `isStopped` exists to prevent — this way
                    // that call is never reached at all for a stopped stream.
                    guard let self, self.streamGeneration == generation else { return }
                    await self.refresh()
                }
            },
            onChangeSet: { [weak self] in
                Task { @MainActor in
                    // Stale-guarded like every other arm here, and for the
                    // reason `onEvent` states: a line already in the pipe when
                    // `stopEvents()` ran is still decoded, and this one would
                    // otherwise launch a subprocess for a runner nobody holds.
                    guard let self, self.streamGeneration == generation else { return }
                    self.refreshChangesInboxSoon()
                }
            },
            onEnd: { [weak self] in
                Task { @MainActor in
                    // Stale: either this stream was deliberately stopped, or
                    // it already lost a race to a newer one. Either way,
                    // touching `eventStream` or arming a retry here would be
                    // acting on behalf of a stream nobody holds anymore.
                    guard let self, self.streamGeneration == generation else { return }
                    self.eventStream = nil
                    self.scheduleRetry()
                    // Only overwrite with the generic "reconnecting" state
                    // for an ordinary drop. `.notInstalled` and a
                    // version-mismatch `.unreachable` are more specific than
                    // "reconnecting" would be, and both just got a wait
                    // measured in minutes, not moments — showing
                    // "reconnecting" over that whole span would say less
                    // than what `refresh()` already determined.
                    if case .notInstalled = self.state { return }
                    if case .unreachable(let reason) = self.state, self.looksVersionMismatch(reason) {
                        return
                    }
                    self.state = .reconnecting(attempt: self.attempt)
                }
            })
        stream.start(binary: binary, environment: environment, host: cliHostArguments)
        eventStream = stream

        // Only a stream that survives is proof the daemon is genuinely back.
        // Resetting `attempt` on every successful `refresh()` instead (the
        // first cut of this) reset it even when the stream that `refresh()`
        // preceded died again immediately after — which turned a flapping
        // daemon into a retry every ~1s forever, spawning two subprocesses a
        // second, rather than a backoff that actually grows.
        //
        // `eventStream != nil` matters as much as the generation match:
        // `onEnd` does not bump `streamGeneration` (only `startEvents` and
        // `stopEvents` do), so between this stream dying and the next
        // `startEvents()` call, the generation alone still matches. Without
        // also checking that the stream is still actually there, this timer
        // would fire mid-backoff and zero `attempt` out from under it — which
        // is exactly finding 5 again, just needing a flappier daemon (three
        // failures instead of one) to reach it, since the armed wait only
        // has to exceed 5s once `attempt` reaches 3.
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(5))
            guard let self, self.streamGeneration == generation, self.eventStream != nil else {
                return
            }
            self.attempt = 0
        }
    }

    func stopEvents() {
        // Invalidates this stream's `onEnd` before it can even fire: `stop()`
        // below terminates the process asynchronously, and its termination
        // handler runs regardless of whether the stop was deliberate.
        streamGeneration += 1
        // A backoff already armed at the moment of a stop must not be left
        // ticking: it would fire later regardless, calling `refresh()` and
        // `startEvents()` against a client that was just told to be quiet —
        // resurrecting the very thing this call exists to stop. `.onDisappear`
        // relies on this: a window closing mid-backoff must leave nothing
        // running behind it.
        //
        // Nilled out, not just cancelled: a cancelled task skips both places
        // that null this slot on completion (each gated on `!Task.isCancelled`,
        // precisely so a superseded cycle cannot clobber whatever replaced
        // it) — so without this, `retryTask` is left permanently non-nil,
        // pointing at a task that will never run again. `scheduleRetry()`'s
        // idempotency guard reads non-nil as "a live retry is armed" and
        // every later call from `onEnd` or `refresh()` would silently no-op
        // forever: `state` keeps reading `.reconnecting`/`.unreachable`, but
        // nothing is actually counting down. The invariant this relies on —
        // a cancelled retry always leaves the slot nil — has to hold at
        // every site that cancels, not just the ones that go on to replace
        // it in the same breath.
        //
        // Set before either of those, and load-bearing beyond them: a
        // `refresh()` already in flight has no cancellation check between
        // it and its own `scheduleRetry()` call, so nilling `retryTask`
        // alone leaves that guard reading "idempotent" instead of "stopped".
        // See `isStopped`'s own doc comment.
        isStopped = true
        retryTask?.cancel()
        retryTask = nil
        eventStream?.stop()
        eventStream = nil
    }

    /// Fold one pushed change into the fleet.
    ///
    /// Applied in place rather than triggering a full re-read: a re-read per
    /// event would make a busy fleet slower than the polling this replaced.
    private func apply(_ event: TerminalEvent) {
        for w in fleet.workspaces.indices {
            guard
                let t = fleet.workspaces[w].terminals.firstIndex(where: { $0.id == event.id })
            else { continue }

            fleet.workspaces[w].terminals[t].state = event.state
            fleet.workspaces[w].terminals[t].activity = event.activity
            // What is RUNNING, which is also what the terminal is CALLED.
            //
            // This was missed, and the omission was invisible until the name
            // started being derived from it: the daemon broadcasts the moment a
            // pane's command changes, the app applied the state and the activity
            // out of that event and dropped the command — so a shell you had just
            // run `node` in stayed labeled `shell` until something forced a full
            // re-read. The whole point of pushing events is not needing one.
            fleet.workspaces[w].terminals[t].preset = event.preset
            // What can be switched to a chat, which is also what `⌃B a`
            // checks before it will even try.
            //
            // This was missed the same way `preset` was missed above, and
            // the omission was the branch's own headline bug surviving its
            // own fix: the daemon pushes `chatCapable` the instant a shell
            // pane's foreground process becomes `codex`, but until this line
            // existed the app applied everything else out of that event and
            // dropped this field — so `canSwitchPaneMode` stayed false
            // forever, since this app is push-only and never re-fetches a
            // terminal it already knows.
            fleet.workspaces[w].terminals[t].chatCapable = event.chatCapable
            // Same reason as `chatCapable` above, one field later: without
            // this, a terminal pushed into `exited` here reads as a clean
            // exit — `Status` sees a `nil` exit code, which is deliberately
            // never a failure — until some later full refresh happens to
            // backfill it. A failed build must not wait on that to be seen.
            fleet.workspaces[w].terminals[t].exitCode = event.exitCode
            fleet.workspaces[w].terminals[t].exitSignal = event.exitSignal
            // The daemon has always sent this; it was never applied here,
            // which is the same omission a third time — a live-pushed
            // Working or Blocked row kept showing whatever `statusDuration`
            // last got from a full refresh instead of what just changed.
            fleet.workspaces[w].terminals[t].activitySince = event.activitySince
            // Same reason as `exitCode` above, one tick later: the moment a
            // row goes Blocked over this event is exactly the moment it
            // needs the turn clock and the question to be current, not
            // whatever a later refresh happens to backfill.
            fleet.workspaces[w].terminals[t].turnStartedAt = event.turnStartedAt
            fleet.workspaces[w].terminals[t].blockedQuestion = event.blockedQuestion
            // The fields whose whole job is "what is it doing RIGHT NOW".
            // Applied from the push rather than waited on, because a row that
            // only arrived with a full refresh would always be describing the
            // previous minute — which for these lines is the same as not
            // sending them. `line` is the one that moves most often of all: a
            // task completing takes `3/7` to `4/7` while nothing else about
            // the pane changes.
            fleet.workspaces[w].terminals[t].feed = event.feed
            fleet.workspaces[w].terminals[t].line = event.line
            fleet.workspaces[w].terminals[t].subagents = event.subagents
            // And the field with the most expensive omission: without this a
            // row whose agent just died kept a clean `Done` tick until
            // something else happened in that pane, which for a dead agent is
            // never.
            fleet.workspaces[w].terminals[t].turnFailed = event.turnFailed

            let terminal = fleet.workspaces[w].terminals[t]
            Notifier.shared.report(terminal: terminal, workspace: fleet.workspaces[w].task)
            reapIfExited(terminal)
            return
        }

        // A terminal we have never seen: created elsewhere, or created here
        // before the first read finished. Only a full read can place it in a
        // workspace, so ask for one.
        Task { await refresh() }
    }

    /// Remove a terminal whose process is gone.
    ///
    /// A terminal IS its process. When that exits — cleanly or not — there is
    /// nothing left to show, so the row goes rather than becoming a dead entry
    /// you have to dismiss. `error` counts too: a terminal that never started
    /// has even less to look at than one that stopped.
    ///
    /// `lost` deliberately does not. That is the one state where Far Cooler does
    /// not know what happened, and quietly deleting the evidence is the
    /// opposite of what it should do.
    /// Terminals already offered the chat, so the offer is made once each.
    private var openedAsChat: Set<String> = []

    /// Open a detected agent as a chat, if that is what the user prefers.
    ///
    /// Once per terminal, tracked by id: a user who switches straight back to
    /// the terminal must not be dragged into the chat again on the next refresh
    /// two hundred milliseconds later. The preference sets a default, and a
    /// default that cannot be overruled is a policy.
    private func openAsChatIfPreferred(_ terminal: Terminal) {
        guard Preferences.shared.preferChatMode else { return }
        guard terminal.canSwitchPaneMode, !terminal.isAgentPane else { return }
        guard !openedAsChat.contains(terminal.id) else { return }
        openedAsChat.insert(terminal.id)

        Task {
            _ = await setPaneMode(terminal.short, mode: "agent")
            await refresh()
        }
    }

    private func reapIfExited(_ terminal: Terminal) {
        guard Preferences.shared.autoRemoveExited else { return }
        let kind = StateKind.parse(terminal.state)
        guard kind == .exited || kind == .error else { return }
        guard !reaped.contains(terminal.id) else { return }
        reaped.insert(terminal.id)

        Task {
            _ = await run(["terminal", "remove", terminal.short], background: true)
            Notifier.shared.forget(terminal.id)
            VisitLog.shared.forget(terminal.id)
            await refresh()
        }
    }

    // MARK: - Commands

    /// Has a fleet ever been read successfully?
    ///
    /// Without this, "we could not read the fleet" and "there are no
    /// workspaces" look identical to the UI, because a failed read leaves the
    /// last value in place — and the first value is empty. A user who had just
    /// created a workspace was shown the new-user empty state, which is the
    /// most misleading thing the app could have said.
    @Published private(set) var hasLoaded = false

    func refresh() async {
        // `runRaw`, not `run`: the failure message comes back from THIS
        // call directly rather than being read out of `lastError` after the
        // fact, where a concurrent command's own failure could have
        // overwritten it between that call resuming and this line running.
        let (maybeData, failureMessage) = await runRaw(
            ["workspace", "list", "--json"], background: true)
        guard let data = maybeData else {
            let reason = failureMessage ?? "Couldn’t reach this runner."
            lastError = reason
            if looksNotInstalled(reason) {
                state = .notInstalled
            } else {
                state = .unreachable(reason: reason)
            }
            // Not a failure to retry forever at full speed (`.notInstalled`
            // and a version mismatch are not fixed by retrying at all), but
            // not one to never check again either — `scheduleRetry()` reads
            // `state` itself and picks the slow cadence for those two cases.
            scheduleRetry()
            return
        }
        do {
            fleet = try JSONDecoder().decode(Fleet.self, from: data)
            hasLoaded = true
            // Diff status for the whole sidebar, in one more call. Cheap by
            // construction: the daemon answers it from counts it already holds
            // plus a two-syscall gate per worktree, so a fleet where nothing is
            // happening costs nothing to keep on screen.
            Task { await self.refreshChangesInbox() }
            lastError = nil
            // Read before overwriting: `onReconnect` fires for a genuine
            // transition into `.connected`, not for a read that merely
            // confirms a connection that was already up — the common case,
            // since `refresh()` has some thirty call sites and most of them
            // run while everything is fine.
            let justReconnected = state != .connected
            state = .connected
            if justReconnected {
                // Every stream this runner was carrying died with the link.
                //
                // `onReconnect` re-reads repositories, roots and layouts, and
                // for a long time that was mistaken for "the runner is back".
                // It is not: a terminal pane and an agent chat each own a
                // `farcooler … ` subprocess of their own, and those exited when
                // ssh did. Nothing restarted them, so a remote runner coming
                // back left every pane on screen frozen at the last byte it
                // received before the drop — the panes were dead and the app
                // said nothing, because as far as it knew it was connected.
                linkGeneration += 1
                onReconnect?()
                // Which build is on the other end of the link that just came
                // up. Cleared first: whatever was read before belongs to the
                // previous link, and a daemon that went away and came back is
                // exactly the case where it could have been replaced.
                //
                // Once per link rather than per read, because a daemon cannot
                // change build without going away — and going away is what
                // ends the event stream, which is what brings us back here. So
                // this costs one extra round trip per reconnection, not one
                // per `refresh()`, of which there are some thirty call sites.
                //
                // Detached, like `refreshChangesInbox()` above and for the same
                // reason: this is news for a dot in the sidebar, and awaiting
                // it here would put an ssh round trip in front of every
                // reconnection before the fleet could be drawn.
                daemonBuild = nil
                daemonBuildUnreadable = false
                Task { await self.readDaemonBuild() }
            }
            // Reap on every read, not only on events. A terminal that exited
            // while the app was closed produces no event to react to, so
            // without this the first thing you see on launch is exactly the
            // clutter auto-removal exists to prevent.
            for workspace in fleet.workspaces {
                for terminal in workspace.terminals {
                    reapIfExited(terminal)
                    openAsChatIfPreferred(terminal)
                }
            }
        } catch {
            // Show the daemon's own output, truncated. A decode failure is
            // almost always something unexpected on stdout, and the first line
            // of it says what.
            //
            // The output ALONE, with no sentence in front of it. Both spellings
            // used to lead with "Could not read the fleet", which is verbatim
            // the heading `ContentView.fleetPlaceholder` draws directly above
            // this — so the screen said it twice, the second time colon-spliced
            // onto a dump of the CLI's stdout. Every surface that renders this
            // now supplies its own sentence and puts these words in a
            // `DetailBox` beneath it, so a prefix here is one sentence too
            // many, and joining it to the CLI's words with a colon is the join
            // `e0f72df` took out of the phone.
            let sample = String(data: data.prefix(200), encoding: .utf8)?
                .trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
            lastError = sample.isEmpty ? error.localizedDescription : sample
            state = .unreachable(reason: lastError ?? "Could not read the fleet.")
            scheduleRetry()
        }
    }

    // MARK: - Which build is answering

    /// Ask this runner which daemon is on the other end, and whether it is the
    /// one this app ships.
    ///
    /// `status --json`, which is what `AboutSheet` already reads for this Mac
    /// and what the CLI prints MISMATCH from. Three reasons it is the right
    /// source rather than `host probe`:
    ///
    /// - It reports the RUNNING daemon. `host probe` reads
    ///   `~/.local/bin/farcoolerd --version` over ssh, which is the binary on
    ///   disk — and a runner whose binary was replaced while its service kept
    ///   running the old process is stale in the way that actually costs you a
    ///   feature, while looking current to the probe.
    /// - The comparison is made where both stamps are known. `buildsMatch` is
    ///   `host_facts.daemon_version == farcooler_protocol::BUILD` inside the
    ///   CLI this app bundles, so "current" means "built with the app you are
    ///   holding" for a remote runner and this Mac alike, without the app
    ///   deriving anything from two strings.
    /// - It never changes what it is measuring. `connect_to` dials and
    ///   reports; only `daemon ensure` replaces anything. A detector that
    ///   quietly fixed what it found would make this whole file pointless.
    private func readDaemonBuild() async {
        // `runRaw`, and its message dropped on the floor: a status read that
        // failed is news for this one dot and nothing else, and `run()` would
        // put it in `lastError` — the window's error banner — where a
        // background poll's failure has no business being. `background: true`
        // for the same reason it is set on `refresh()`: nothing here should
        // toggle `busy` and re-evaluate every terminal surface in the app.
        let (data, _) = await runRaw(["--json", "status"], background: true)
        guard let data,
            let body = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let version = body["daemonVersion"] as? String
        else {
            // No guess either way. `daemonSkew` reads this as `.unknown`,
            // which offers nothing and claims nothing — see its own doc for
            // why silence beats a dot nobody can act on.
            daemonBuildUnreadable = true
            return
        }
        daemonBuildUnreadable = false
        daemonBuild = DaemonBuild(
            version: version,
            // Absent means a CLI or daemon from before the field existed,
            // which cannot have been built with this app — the opposite
            // default from `AboutSheet`'s, which is reading a pair that ships
            // together and has no such case.
            matches: body["buildsMatch"] as? Bool ?? false,
            platform: body["platform"] as? String ?? "")
    }

    /// Replace the daemon on this runner with the build this app ships.
    ///
    /// **Never call this on a schedule, on a reconnection, or on the way to
    /// something else.** It restarts `farcoolerd`, and a restart discards every
    /// agent conversation on that runner — see `DaemonSkew` for where that is
    /// established in the daemon's own source. The only caller is
    /// `DaemonUpdateCard`, after a person has read what it costs and pressed
    /// the button that says so.
    ///
    /// Two mechanisms, because there are two kinds of runner and they are
    /// updated by different things:
    ///
    /// - **A remote runner** is updated by `runner install`, which copies this
    ///   app's `farcooler` and `farcoolerd` over ssh, verifies their SHA-256 on
    ///   the far side, and restarts the service. That is the same command
    ///   Settings ▸ Runners spells "Reinstall".
    /// - **This Mac** is not installed onto at all. Its daemon lives inside the
    ///   app bundle, so bringing it up to date means stopping whatever holds
    ///   the socket and starting the bundled one — `daemon ensure`, through
    ///   `LocalDaemon`, which is the same call the app makes at launch.
    func updateDaemon() async -> DaemonUpdateOutcome {
        if target.isEmpty {
            if let problem = await LocalDaemon.shared.ensure().problem {
                return .failed(problem)
            }
        } else {
            let result = await Runners.shared.install(target)
            guard result.ok else { return .failed(result.output) }
        }

        // The link this client held was to a daemon that has just been stopped,
        // so nothing is on the other end of it. `reconnectNow()` is the same
        // path a click on a trouble dot takes: it refreshes, restarts the event
        // stream, and — through `refresh()`'s reconnection branch — re-reads
        // which build is now answering, which is what makes the dot go quiet
        // without anybody asking it to.
        reconnectNow()
        return .updated
    }

    @Published var repositories: [Repository] = []

    func refreshRepositories() async {
        guard let data = await run(["repo", "list", "--json"], background: true) else { return }
        repositories = (try? JSONDecoder().decode(RepositoryList.self, from: data))?.repositories ?? []
    }

    /// Allowlisted roots, so the app can tell whether a chosen repository is
    /// already covered by one.
    @Published var roots: [RepositoryRoot] = []

    func refreshRoots() async {
        guard let data = await run(["root", "list", "--json"], background: true) else { return }
        roots = (try? JSONDecoder().decode(RootList.self, from: data))?.roots ?? []
    }

    /// Allowlist a directory. Returns the daemon's message, or nil on success.
    func addRoot(_ path: String) async -> String? {
        await runReportingError(["root", "add", path])
    }

    /// Register a git repository that sits inside an allowlisted root.
    func registerRepository(_ path: String) async -> String? {
        let error = await runReportingError(["repo", "register", path])
        await refreshRepositories()
        return error
    }

    /// What asking the daemon to remove a root came back with.
    enum RemoveRootResult {
        case ok
        /// `confirm` did not match the root's own name. Unlike
        /// `RemoveWorktreeResult`'s identical case, this should not happen in
        /// ordinary use — the caller always supplies the root's own name, not
        /// something a user typed — so it means the sheet's own idea of the
        /// name and the daemon's disagree, which is a bug worth surfacing
        /// rather than an expected turn in the flow.
        case confirmationRequired
        case failed(String)
    }

    /// Remove an allowlisted root, and with it every repository registered
    /// under it. `confirm` must be the root's own folder name — the daemon
    /// checks it server-side regardless of what is passed here, the same way
    /// `removeWorktree`'s check is a courtesy for the same reason.
    ///
    /// Touches nothing on disk, same as the CLI's own `root remove` promises.
    @discardableResult
    func removeRoot(_ id: String, confirm: String) async -> RemoveRootResult {
        let before = lastError
        guard await run(["root", "remove", id, "--confirm", confirm]) != nil else {
            let message = lastError ?? "command failed"
            if message.localizedCaseInsensitiveContains("confirmation") {
                lastError = before
                return .confirmationRequired
            }
            return .failed(message)
        }
        await refreshRepositories()
        await refreshRoots()
        return .ok
    }

    /// Everything a task needs, from one sentence.
    ///
    /// Creates the worktree, launches an agent in it, waits for the agent to
    /// actually be ready, and hands it the description as its first message.
    ///
    /// The waiting is the interesting part. An agent takes several seconds to
    /// boot, and text typed into it before then is swallowed by whatever it
    /// draws over the top. Rather than guessing a delay, this waits for the
    /// daemon to report the agent as IDLE — the same activity detection the
    /// sidebar uses. "Ready for input" is exactly what idle means, so the
    /// signal already existed.
    ///
    /// Returns the new workspace, so the caller can select it immediately
    /// rather than after the agent has finished starting.
    // MARK: - Tiling

    /// Each workspace's groups, keyed by workspace id.
    ///
    /// Held here rather than in a view, because the daemon owns it and three
    /// things change it: this app, the CLI, and agents driving the CLI. A layout
    /// in view state would be a fourth opinion.
    @Published var layouts: [String: [PaneGroup]] = [:]

    func activeGroup(_ workspace: String) -> PaneGroup? {
        let groups = layouts[workspace] ?? []
        return groups.first { $0.isActive } ?? groups.first
    }

    /// Which group holds a terminal, if any.
    func group(holding terminal: String, in workspace: String) -> PaneGroup? {
        (layouts[workspace] ?? []).first { $0.terminals.contains(terminal) }
    }

    /// Mark a pane focused locally, before the daemon has been asked.
    ///
    /// Every daemon action here spawns a `farcooler` subprocess, which connects
    /// over a socket and runs `tmux select-pane`. The focus ring, the header
    /// tint and the keyboard claim are all driven by `PaneRect.focused` — a fact
    /// the DAEMON reports — so none of them moved until that whole round trip
    /// finished: locally a fork, an exec and a socket connect, and over ssh all
    /// of that plus the link. That lag is what the review noticed.
    ///
    /// So the answer is assumed and then confirmed. tmux remains the only
    /// authority — the reply replaces this wholesale a moment later, and a
    /// FAILED call re-reads rather than leaving the assumption standing, which
    /// is the one new way this can be wrong. See `focusPane`.
    ///
    /// The pane's group comes forward with it, because `layout focus` brings a
    /// layout to the front on the daemon side too; assuming the focus without
    /// the group would show a ring on a pane in a layout that is not on screen.
    func assumeFocus(_ terminal: String, in workspace: String) {
        guard var groups = layouts[workspace],
            let index = groups.firstIndex(where: { $0.terminals.contains(terminal) })
        else { return }

        for g in groups.indices {
            groups[g].active = g == index
            for p in groups[g].panes.indices {
                groups[g].panes[p].focused = g == index && groups[g].panes[p].id == terminal
            }
        }
        layouts[workspace] = groups
    }

    // MARK: - Layout commands
    //
    // One method per CLI subcommand, and nothing more. Each is a single line over
    // `layout(_:_:_:)`, which exists so the reply — always the workspace's whole
    // layout — is applied in exactly one place. The value of naming them anyway is
    // that the argument order and the flag spellings live here rather than being
    // written out at each call site, which is where the last set of them drifted.

    /// A new terminal beside an existing pane. tmux's `split-window`.
    ///
    /// The only layout command that reads the fleet afterwards, because it is
    /// the only one that creates a terminal. A layout is rectangles keyed by
    /// terminal id; the terminal itself arrives in `fleet`, and until it does
    /// the new pane is a rectangle this app can draw nothing into. The daemon
    /// announces it too — see `layout.split` in `rpc.rs` — but that is for the
    /// OTHER clients. Waiting for our own announcement to come back around the
    /// event stream is a round trip spent looking at an empty pane.
    @discardableResult
    func split(
        _ workspace: Workspace, beside terminal: String?, side: TileDirection,
        preset: String = "shell"
    ) async -> [PaneGroup] {
        var rest = terminal.map { [$0] } ?? []
        rest += ["--side", side.rawValue, "--preset", preset]
        let groups = await layout(workspace, ["split"], rest)
        await refresh()
        return groups
    }

    /// Move a pane against another, on an edge. The drag and drop.
    ///
    /// Works across layouts — the pane leaves the one it was in — which is why
    /// this single call covers both halves of the gesture: rearranging panes
    /// within a layout and pulling a terminal in from another one.
    @discardableResult
    func movePane(
        _ terminal: String, onto target: String, side: TileDirection, in workspace: Workspace
    ) async -> [PaneGroup] {
        await layout(workspace, ["move"], [terminal, target, "--side", side.rawValue])
    }

    @discardableResult
    func applyPreset(_ preset: TilePreset, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["preset"], [preset.rawValue])
    }

    @discardableResult
    func cycleLayout(_ workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["cycle"])
    }

    /// Focus a pane, which also brings its layout to the front.
    ///
    /// Assumed locally first — see `assumeFocus` for why that is the whole fix
    /// for a focus ring that used to arrive a round trip late.
    @discardableResult
    func focusPane(_ terminal: String, in workspace: Workspace) async -> [PaneGroup] {
        assumeFocus(terminal, in: workspace.id)
        return await confirmed(workspace, ["focus"], [terminal])
    }

    @discardableResult
    func focusPane(step: String, in workspace: Workspace) async -> [PaneGroup] {
        // `--next`/`--prev` step through a pane order the app already holds, so
        // the target is knowable here and the assumption is as safe as it is for
        // a pane named outright.
        if let group = activeGroup(workspace.id), !group.panes.isEmpty,
            let current = group.panes.firstIndex(where: \.focused)
        {
            let delta = step == "--prev" ? -1 : 1
            let next = (current + delta + group.panes.count) % group.panes.count
            assumeFocus(group.panes[next].id, in: workspace.id)
        }
        return await confirmed(workspace, ["focus"], [step])
    }

    @discardableResult
    func focusPane(number: Int, in workspace: Workspace) async -> [PaneGroup] {
        if let group = activeGroup(workspace.id), number >= 1, number <= group.panes.count {
            assumeFocus(group.panes[number - 1].id, in: workspace.id)
        }
        return await confirmed(workspace, ["focus"], ["--pane", "\(number)"])
    }

    /// Run a focus command and make sure the local copy ends up telling the
    /// truth either way.
    ///
    /// This is the one new failure mode optimistic focus introduces, so it is
    /// handled in one place rather than at each of the three call sites. The
    /// caller has already written an assumption into `layouts`; if the daemon
    /// never answers, that assumption is a ring drawn on a pane which never got
    /// focus, and nothing would ever correct it. So a failure re-reads.
    ///
    /// `layoutOrNil` rather than `layout` because only the former can tell the
    /// difference: `layout` answers a failure with the local copy, which is now
    /// the copy carrying the assumption.
    private func confirmed(
        _ workspace: Workspace, _ path: [String], _ rest: [String]
    ) async -> [PaneGroup] {
        if let groups = await layoutOrNil(workspace, path, rest, background: true) {
            return groups
        }
        await refreshLayout(workspace)
        return layouts[workspace.id] ?? []
    }

    @discardableResult
    func zoomPane(_ terminal: String?, in workspace: Workspace, off: Bool = false)
        async -> [PaneGroup]
    {
        await layout(workspace, ["zoom"], (terminal.map { [$0] } ?? []) + (off ? ["--off"] : []))
    }

    @discardableResult
    func swapPanes(_ a: String, _ b: String, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["swap"], [a, b])
    }

    @discardableResult
    func resizePane(
        _ terminal: String, side: TileDirection, cells: Int, in workspace: Workspace
    ) async -> [PaneGroup] {
        await layout(
            workspace, ["resize"], [terminal, "--side", side.rawValue, "--cells", "\(cells)"])
    }

    /// Pull a pane into a layout of its own. tmux's `break-pane`.
    @discardableResult
    func breakPane(_ terminal: String?, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["break"], terminal.map { [$0] } ?? [])
    }

    @discardableResult
    func renameLayout(_ name: String, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["rename"], [name])
    }

    /// Tell tmux how big the view showing this layout is, in cells.
    ///
    /// The one thing the app is authoritative about, because it is the only thing
    /// tmux cannot see: how much screen there is. Everything else flows back the
    /// other way — tmux lays out into this and reports where the panes landed.
    @discardableResult
    func viewport(columns: Int, rows: Int, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["viewport"], ["\(columns)", "\(rows)"])
    }

    /// Show a different layout: by tmux window id, by name, by number, or `--next`.
    @discardableResult
    func selectLayout(_ group: String, in workspace: Workspace) async -> [PaneGroup] {
        await layout(workspace, ["select"], [group])
    }

    func refreshLayout(_ workspace: Workspace) async {
        guard let data = await run(["layout", "show", workspace.short, "--json"]) else { return }
        guard let list = try? JSONDecoder().decode(PaneGroupList.self, from: data) else { return }
        layouts[workspace.id] = list.groups
    }

    /// Every layout the fleet has, read once.
    ///
    /// One call per workspace rather than one for the fleet: `layout show` is
    /// scoped to a workspace, and a fleet-wide read would be a method that exists
    /// only for a first paint. Events carry every change after this.
    func refreshLayouts() async {
        for workspace in fleet.workspaces {
            await refreshLayout(workspace)
        }
    }

    /// Run a layout command and apply the groups it returns.
    ///
    /// `path` is the subcommand, `rest` its arguments; the workspace goes between
    /// them, which is where every one of these commands wants it. Spelled out
    /// rather than inserted at a fixed index — a previous version inserted it at
    /// position 2, which is right for a one-word subcommand and wrong for a
    /// two-word one, so half the commands silently acted on the wrong thing.
    ///
    /// The reply is the workspace's whole layout, so the local copy is replaced
    /// rather than patched — and the event that follows says the same thing,
    /// which is what keeps a second client in step.
    ///
    /// `background` skips the `busy` toggle, which is a `@Published` change that
    /// re-evaluates the whole view tree — terminal surface included — on every
    /// call. Only the focus paths pass true: a split or a preset change
    /// genuinely is the app doing something the user should see it doing, and
    /// `busy` is how it says so.
    @discardableResult
    func layout(
        _ workspace: Workspace, _ path: [String], _ rest: [String] = [],
        background: Bool = false
    ) async -> [PaneGroup] {
        await layoutOrNil(workspace, path, rest, background: background)
            ?? layouts[workspace.id] ?? []
    }

    /// The same call, reporting failure instead of hiding it.
    ///
    /// `layout` answers a failure with the local copy, which is right for its
    /// callers — a split that did not happen should leave the arrangement on
    /// screen alone — and useless to the focus paths, which have just written an
    /// ASSUMPTION into that copy. Handed it back, they cannot tell a daemon that
    /// agreed from a daemon that never answered, so a focus against an
    /// unreachable runner would leave the ring on a pane that never got it.
    ///
    /// `nil` means the command did not produce a layout, whether it failed to
    /// run or answered with something undecodable.
    private func layoutOrNil(
        _ workspace: Workspace, _ path: [String], _ rest: [String] = [],
        background: Bool = false
    ) async -> [PaneGroup]? {
        let command = ["layout"] + path + [workspace.short] + rest
        guard let data = await run(command + ["--json"], background: background),
            let list = try? JSONDecoder().decode(PaneGroupList.self, from: data)
        else { return nil }
        layouts[workspace.id] = list.groups
        return list.groups
    }

    /// Branches in a project that work could be resumed on.
    func branches(project: String) async -> [BranchInfo] {
        guard let data = await run(["workspace", "branches", project, "--json"]) else { return [] }
        return (try? JSONDecoder().decode(BranchList.self, from: data))?.branches ?? []
    }

    /// Pick up work that already exists on a branch.
    ///
    /// A branch that is only on a remote gets a local tracking branch, which is
    /// the whole point when the work came from another runner or another
    /// person: pushing back has to go where it came from.
    @discardableResult
    func adoptBranch(project: String, branch: String, agent: String) async -> String? {
        let before = Set(fleet.workspaces.map(\.id))
        _ = await run(["workspace", "adopt", project, branch])
        await refresh()

        guard let workspace = fleet.workspaces.first(where: { !before.contains($0.id) })
        else { return nil }
        _ = await run([
            "terminal", "create", workspace.short, "--preset", agent, "--title", "Agent",
        ])
        await refresh()
        return workspace.id
    }

    func startTask(project: String, description: String, agent: String) async -> String? {
        // This runner's own prefix, read from the fleet it last refreshed — the
        // same value the composer previewed, so the branch that gets made is the
        // branch the user was shown.
        let prefix = fleet.branchPrefix ?? ""
        let branch = await MainActor.run { Branch.slug(from: description, prefix: prefix) }
        // The positional is the worktree's name now rather than a description
        // of the task, which is why a whole prompt is cut down before it is
        // sent: it is about to become a directory, and the composer previewed
        // the path it makes.
        let name = await MainActor.run { Branch.title(from: description) }

        let before = Set(fleet.workspaces.map(\.id))
        // `--no-terminal`, because this creates its own agent terminal a few
        // lines below. Without it a task would come up with an unused shell
        // sitting beside the agent that is doing the work.
        _ = await run([
            "workspace", "create", project, name, "--branch", branch, "--no-terminal",
        ])
        await refresh()

        guard let workspace = fleet.workspaces.first(where: { !before.contains($0.id) }) else {
            return nil
        }

        _ = await run([
            "terminal", "create", workspace.short, "--preset", agent, "--title", agent,
        ])
        await refresh()

        guard let terminal = fleet.workspaces
            .first(where: { $0.id == workspace.id })?
            .terminals.first(where: { $0.preset == agent || $0.title == agent })
        else { return workspace.id }

        // Up to a minute: a cold agent on a slow machine is not a failure.
        for _ in 0..<120 {
            try? await Task.sleep(for: .milliseconds(500))
            // `try?` swallows `Task.sleep`'s `CancellationError`, so a
            // cancelled `startTask` — the runner it targets was just
            // removed — would otherwise fall straight through into another
            // `refresh()` and, once the agent went idle, `send()` the task
            // description into a runner nothing holds a client for anymore.
            // Checked explicitly rather than relying on the sleep to throw.
            guard !Task.isCancelled else { return workspace.id }
            await refresh()
            let current = fleet.workspaces
                .first(where: { $0.id == workspace.id })?
                .terminals.first(where: { $0.id == terminal.id })
            guard let current else { return workspace.id }
            if current.agent == .idle {
                await send(terminal: current.short, text: description)
                return workspace.id
            }
            // It asked something before we got a word in — a trust prompt, or a
            // resume dialog. Stop rather than typing a task description into a
            // yes/no question.
            if current.agent == .blocked { return workspace.id }
        }
        return workspace.id
    }

    /// Type text into a terminal and press return.
    func send(terminal: String, text: String) async {
        _ = await run(["terminal", "send", terminal, text], background: true)
        // Return as a separate keystroke: an agent's composer treats a newline
        // inside pasted text as a line break, not as submit.
        _ = await run(["terminal", "send-hex", terminal, "0d"], background: true)
    }

    /// What creating a worktree came back with.
    ///
    /// Both halves, because the caller needs both: the daemon's own message so
    /// `NewWorkspaceSheet` can stay open and show it rather than dismissing as
    /// if nothing went wrong, and the workspace that appeared so the window can
    /// go to it. Returning only the failure meant a worktree was created,
    /// the sheet closed, and nothing on screen changed — the new work was
    /// somewhere in the sidebar, collapsed, for you to go and find.
    struct CreatedWorkspace {
        let failure: String?
        /// Nil on failure, and nil in the case where the refresh that followed
        /// cannot say which workspace is the new one. Callers treat that as
        /// "created, but do not move the selection" rather than guessing.
        let workspace: String?
    }

    func createWorkspace(repo: String, task: String, branch: String, base: String) async
        -> CreatedWorkspace
    {
        // Sampled before the create, and diffed after the refresh — the same
        // way `startTask` finds the workspace it just made. The daemon does not
        // report the id it minted, and matching on the name would find the
        // wrong one the second time a name is reused across projects.
        let before = Set(fleet.workspaces.map(\.id))
        let failure = await runReportingError([
            "workspace", "create", repo, task, "--branch", branch, "--base", base,
            // A worktree with nothing running in it is a directory. `shell`
            // rather than an agent, because this is the manual path — `startTask`
            // is the one that starts an agent — and it matches what
            // "New terminal in <project>" already opens in the main checkout.
            "--terminal", "shell",
        ])
        await refresh()
        guard failure == nil else { return CreatedWorkspace(failure: failure, workspace: nil) }
        return CreatedWorkspace(
            failure: nil,
            workspace: fleet.workspaces.first { !before.contains($0.id) }?.id)
    }

    func hideWorkspace(_ workspace: String) async {
        _ = await run(["workspace", "hide", workspace])
        await refresh()
    }

    func unhideWorkspace(_ workspace: String) async {
        _ = await run(["workspace", "unhide", workspace])
        await refresh()
    }

    /// What asking the daemon to remove a worktree came back with.
    enum RemoveWorktreeResult {
        case ok
        /// The daemon needs the typed workspace name because the worktree is
        /// dirty (`DomainError::ConfirmationRequired`). Carries no message:
        /// the sheet has fixed wording for this one specific refusal.
        case confirmationRequired
        /// Refused for any other reason — running terminals, tmux
        /// unavailable, a failed `git worktree remove` — carrying the
        /// daemon's own message so the sheet can show what actually went
        /// wrong instead of guessing "uncommitted work".
        case failed(String)
    }

    /// Remove a worktree. `confirm` must be the workspace's exact task name,
    /// unless the worktree is clean (or its directory is already gone), in
    /// which case it may be empty and is omitted entirely — Task 8 made
    /// `--confirm` optional on the CLI side for exactly this.
    ///
    /// Forwarded rather than checked only here: the daemon refuses a mismatch
    /// itself, so the dialog is a courtesy and the daemon's check is the one
    /// that actually protects the files.
    ///
    /// Distinguishes "confirmation required" from every other refusal the
    /// same way `setPaneMode` distinguishes its own: a non-zero exit alone
    /// does not mean the worktree is dirty, and reporting every refusal —
    /// `RunningProcesses`, `TmuxUnavailable`, a failed `git worktree remove`
    /// — as "there is uncommitted work here" tells the user to type a name
    /// that will never make the real problem go away.
    @discardableResult
    func removeWorktree(_ workspace: String, confirm: String) async -> RemoveWorktreeResult {
        var args = ["workspace", "remove-worktree", workspace]
        if !confirm.isEmpty { args += ["--confirm", confirm] }

        let before = lastError
        guard await run(args) != nil else {
            let message = lastError ?? "command failed"
            if message.localizedCaseInsensitiveContains("confirmation") {
                // Leave the banner clean: this refusal becomes the sheet's
                // own field and callout, not a banner behind it.
                lastError = before
                return .confirmationRequired
            }
            return .failed(message)
        }
        await refresh()
        return .ok
    }

    /// The rendered visible screen, color escapes intact.
    func screen(terminal: String) async -> String {
        guard let data = await run(["terminal", "screen", terminal, "--json"]) else { return "" }
        struct Screen: Decodable { var screen: String }
        return (try? JSONDecoder().decode(Screen.self, from: data))?.screen ?? ""
    }

    func capture(terminal: String, lines: Int = 400) async -> String {
        guard let data = await run(["terminal", "read", terminal, "--lines", "\(lines)"])
        else { return "" }
        return String(data: data, encoding: .utf8) ?? ""
    }

    /// Size one terminal to a viewer's grid.
    ///
    /// Only for a terminal being shown ALONE. A pane's size is a property of the
    /// layout it is in, so calling this on one pane of several resizes the whole
    /// window to that pane's grid and the other panes shrink to fit inside it —
    /// which is what happened, once, when each pane reported its own geometry.
    /// The tiled view calls `viewport` instead, exactly once for the whole view.
    func resize(terminal: String, columns: Int, rows: Int) async {
        _ = await run(["terminal", "resize", terminal, "\(columns)", "\(rows)"])
    }

    /// Files in a worktree matching a partial path, for the composer's `@`
    /// picker.
    ///
    /// DEVIATION, recorded the same way `AgentStream` records its own:
    /// `crates/cli` has no `workspace file-search` subcommand yet, so this is
    /// written against the shape the feature needs — a query in, matching
    /// paths out — and is inert until that subcommand lands. Empty on any
    /// failure rather than surfacing `lastError`: a mention picker that
    /// cannot search is a picker with nothing to show, not a reason to put a
    /// banner over someone's half-typed message.
    func searchFiles(in workspace: Workspace, query: String) async -> [String] {
        guard !query.isEmpty else { return [] }
        guard
            let data = await run([
                "workspace", "file-search", workspace.short, query, "--json",
            ])
        else { return [] }
        struct Result: Decodable { var paths: [String] }
        return (try? JSONDecoder().decode(Result.self, from: data))?.paths ?? []
    }

    /// What asking the daemon to switch a pane's mode came back with.
    enum PaneModeResult {
        case ok
        /// A turn is in flight; switching would cancel it.
        ///
        /// Carries what the CLI printed, which is NOT a description of the
        /// turn however this once read: `DomainError::ConfirmationRequired`
        /// is a unit variant and its whole message is "exact typed
        /// confirmation required". `PaneModeConfirmSheet` writes the sentence
        /// about the turn itself and shows this in a `DetailBox`, because a
        /// refusal string set as the app's own warning is the app appearing
        /// to have said it.
        case confirmationRequired(String)
        case failed(String)
    }

    /// Switch a pane between showing its terminal and showing its agent chat.
    ///
    /// DEVIATION, same shape as `searchFiles` above: `crates/cli` has no
    /// `terminal set-pane-mode` subcommand yet. This is written against the
    /// daemon's own contract for it — a plain success, or a refusal naming
    /// what is in flight — so the confirmation flow above it (`ContentView`)
    /// can be built and reviewed now rather than after the CLI catches up.
    /// The exact wording the daemon will use for a refusal is not settled
    /// either, so the detection here is a case-insensitive substring match
    /// against "confirmation" — a best guess against an undefined wire
    /// format, not a parsed error code, and worth tightening once the real
    /// shape exists.
    @discardableResult
    func setPaneMode(_ terminal: String, mode: String, force: Bool = false) async -> PaneModeResult {
        var args = ["terminal", "set-pane-mode", terminal, mode]
        if force { args.append("--force") }

        let before = lastError
        guard await run(args) != nil else {
            let message = lastError ?? "command failed"
            if message.localizedCaseInsensitiveContains("confirmation") {
                // Leave the banner clean: this refusal becomes a sheet, not a
                // banner, in `ContentView`.
                lastError = before
                return .confirmationRequired(message)
            }
            return .failed(message)
        }
        await refresh()
        return .ok
    }

    /// Create a terminal and return it, identified by difference.
    ///
    /// The create call does not report which record it made, and comparing whole
    /// `Terminal` values does not work — any of them changing activity between the
    /// two reads also looks new. Ids are stable, so the id set is the diff.
    @discardableResult
    func createTerminal(
        in workspace: Workspace, preset: String, title: String
    ) async -> Terminal? {
        let before = Set(workspace.terminals.map(\.id))
        await createTerminal(workspace: workspace.short, preset: preset, title: title)

        // Creation is a tmux window opening, so the record can lag the call.
        for _ in 0..<20 {
            if let found = fleet.workspaces.first(where: { $0.id == workspace.id })?
                .terminals.first(where: { !before.contains($0.id) })
            {
                return found
            }
            try? await Task.sleep(for: .milliseconds(150))
            // Same reasoning as `startTask`'s own loop just above: `try?`
            // alone lets a cancelled caller keep polling — up to 20 more
            // `workspace list` subprocesses against a runner that may have
            // just been removed — instead of stopping the moment it is told to.
            guard !Task.isCancelled else { return nil }
            await refresh()
        }
        return nil
    }

    /// A terminal, which is a tmux window, which is a layout of its own.
    ///
    /// Deliberately NOT `--tile`. A new terminal gets its own layout tab;
    /// putting a pane into an existing arrangement is what `layout split` and
    /// `⌃B %` are for, and they say so at the point of use. Tiling here made
    /// every new terminal a split of whatever happened to be focused, which
    /// also hid the tab strip — with one layout there are no tabs to show —
    /// so the pane appeared to arrive nowhere at all.
    func createTerminal(workspace: String, preset: String, title: String) async {
        _ = await run(["terminal", "create", workspace, "--preset", preset, "--title", title])
        await refresh()
    }

    func restart(terminal: String) async {
        _ = await run(["terminal", "restart", terminal])
        await refresh()
    }

    /// Be rid of a terminal whose pane cannot be found.
    ///
    /// The daemon deletes the record — a lost terminal has no pane, no output
    /// and no exit code, so once it has been acknowledged there is nothing left
    /// for the row to say. Its notification and its place in the switcher go
    /// with it: both point at something that no longer exists.
    func dismissLost(_ terminal: Terminal) async {
        _ = await run(["terminal", "dismiss-lost", terminal.short])
        Notifier.shared.forget(terminal.id)
        VisitLog.shared.forget(terminal.id)
        await refresh()
    }

    /// Tell the daemon a terminal was opened.
    ///
    /// This is what ends `done`, which is defined as finished-and-unseen.
    /// Called on selection, not on appearing in a list: being listed is not
    /// being read, and clearing a notification nobody read is worse than not
    /// sending one.
    func markSeen(_ terminal: String) async {
        _ = await run(["terminal", "seen", terminal], background: true)
    }

    /// The terminals this window last told the runner it was showing, and when.
    ///
    /// Full ids, not the eight-character `short` every other command here takes.
    /// A short id costs the CLI a `terminal.list` round trip to resolve, and this
    /// call runs on a clock — see the `Watching` command in
    /// `crates/cli/src/main.rs`, which takes a whole UUID as it is for exactly
    /// this reason.
    private var watching: [String] = []
    private var watchingSentAt: Date = .distantPast
    /// The renewal in flight, so arming a second one replaces the first rather
    /// than running beside it. Same one-slot rule `retryTask` follows.
    private var watchingTask: Task<Void, Never>?
    /// Cancelled with this client, so a renewal cannot outlive the runner it
    /// was talking about.
    private var resignObserver: AnyCancellable?

    /// How often a standing claim of attention is renewed.
    ///
    /// Three seconds, which is this app's one existing cadence — see
    /// `changesInboxFloor` beside it and the phone's poll — and comfortably
    /// inside the ten seconds the runner believes a claim for
    /// (`WATCHED_TTL_MS` in `crates/daemon/src/watch.rs`). Two consecutive
    /// failures are therefore survivable without a pane somebody is plainly
    /// watching starting to buzz them.
    private static let watchingFloor: TimeInterval = 3

    /// Tell this runner which panes are in front of the person right now, so it
    /// does not push a notification about an agent they are already reading.
    ///
    /// The complaint in the owner's words: a codex pane open, a question asked,
    /// an answer given, and a buzz on the wrist about a reply already on screen.
    /// `markSeen` above cannot fix that and never could — it says a pane HAS
    /// been read, on the fleet event AFTER the turn ended, by which time the
    /// push has crossed the relay. This says a pane IS being read, in advance,
    /// so it is already true at the moment the turn ends.
    ///
    /// Suppression on the runner is per terminal and across every device,
    /// because it is one person: this Mac saying its window can see a pane is
    /// what silences the watch on their wrist. That is the whole point.
    ///
    /// The WHOLE visible set each time, replacing whatever this client said
    /// last, so clicking from one pane to the next releases the first in the
    /// same call that claims the second. An empty array is a real and important
    /// call — "I am looking at nothing" — which is what leaving the app sends.
    ///
    /// **A timer, in the app that deliberately deleted its timers.** The
    /// distinction is what the clock is for. `EventStream` replaced POLLING —
    /// asking a quiet runner over and over what it already told us — and this
    /// asks nothing: it is an assertion with an expiry, and an assertion that is
    /// not renewed has to lapse or a crashed window would silence a terminal
    /// forever. It also cannot be hung off fleet events, which was tried on
    /// paper first: a working agent whose signal line happens not to move for
    /// ten seconds — one long tool call — would let the claim expire in exactly
    /// the seconds before it finishes, which is the one moment this has to be
    /// right. It runs only while the app is frontmost with panes on screen, and
    /// costs one `farcooler` invocation per three seconds over the ssh control
    /// socket the CLI already keeps open — the same order as the
    /// `changes inbox` read this app already makes on the same clock while an
    /// agent works.
    func reportWatching(_ terminals: [String]) {
        // `"watching"` is `farcooler_protocol::capability::WATCHING`. A runner
        // that predates it withholds nothing and notifies exactly as it always
        // did — but this renews itself every few seconds for as long as a pane
        // is on screen, so without the check it would spawn a `farcooler`
        // invocation on that clock forever to be refused the same way each
        // time. `daemonBuild` is nil until the first status read lands, which
        // reads as "not yet" and is retried by the next claim.
        guard daemonBuild?.can("watching") ?? false else { return }
        // A window that is not frontmost is not showing anybody anything, and
        // saying otherwise would suppress the notification that exists for
        // precisely that case. `ContentView.markVisibleSeen` gates on this too;
        // it is repeated here because the renewal below fires on its own clock,
        // long after the call that armed it.
        let claim = NSApp.isActive ? terminals : []
        let changed = claim != watching
        watchingTask?.cancel()
        watchingTask = nil

        if changed || Date().timeIntervalSince(watchingSentAt) >= Self.watchingFloor {
            // Stamped before the call and stamped even when it fails, on the
            // same terms as `changesInboxReadAt`: a runner that refuses this in
            // a millisecond must not thereby be asked far more often than one
            // that answers it. The floor is a floor on ATTEMPTS.
            watching = claim
            watchingSentAt = Date()
            Task { [weak self] in
                _ = await self?.run(["terminal", "watching"] + claim, background: true)
            }
        }

        // Nothing left to renew. A released claim is the end of it — the runner
        // forgets it on its own within `WATCHED_TTL_MS` even if this last call
        // never lands.
        guard !claim.isEmpty else { return }
        watchingTask = Task { [weak self] in
            try? await Task.sleep(for: .seconds(Self.watchingFloor))
            guard !Task.isCancelled else { return }
            self?.reportWatching(terminals)
        }
    }

    /// Give the claim back the moment the window stops being frontmost.
    ///
    /// Rather than waiting for it to age out on the runner. An agent finishing
    /// while you are in another app is precisely what the notification exists
    /// for, so the seconds between switching away and the claim expiring are
    /// seconds this feature would spend swallowing the notification it is least
    /// entitled to swallow.
    ///
    /// Observed here rather than in the view, unlike `didBecomeActive`, because
    /// the view's own hook is `markVisibleSeen`, which begins by refusing to do
    /// anything at all when the app is not active — so there is no hook there
    /// that resigning could reach. This is also what makes the renewal above
    /// safe to leave running: whichever of the two notices first, the claim
    /// goes.
    private func watchResignations() {
        resignObserver = NotificationCenter.default
            .publisher(for: NSApplication.didResignActiveNotification)
            .sink { [weak self] _ in
                Task { @MainActor in self?.reportWatching([]) }
            }
    }

    /// Delete a terminal's record. Refused by the daemon while it is running.
    func removeTerminal(_ terminal: String) async {
        _ = await run(["terminal", "remove", terminal])
        await refresh()
    }

    func stop(terminal: String) async {
        _ = await run(["terminal", "stop", terminal])
        await refresh()
    }

    // MARK: - Subprocess

    /// Run a command; on failure, set `lastError` and hand back `nil`.
    ///
    /// A thin wrapper over `runRaw`, kept for the roughly thirty call sites
    /// in this file that only ever wanted the data-or-banner behavior. The
    /// one caller that needs its OWN failure's exact words — `refresh()`,
    /// which decides `.unreachable`'s reason and whether this is a runner
    /// that needs installing — calls `runRaw` directly instead. See there.
    @discardableResult
    // ---- changes ----
    //
    // Every one of these is the same CLI the terminal rows already go through.
    // A review surface the app could reach but an agent could not would make the
    // one workflow this product is about the one thing nobody can automate.

    func changesJSON(_ args: [String]) async -> Data? {
        let (data, message) = await runRaw(args, background: true)
        // Kept rather than dropped. Swallowing this is what made a runner
        // running an older daemon look like a worktree with no changes: the call
        // failed, the pane rendered an empty diff, and nothing anywhere said why.
        changesError = data == nil ? message : nil
        if data != nil { changesSupported = true }
        return data
    }

    /// Diff status for every worktree on this runner, in one call.
    ///
    /// One call rather than one per row: the daemon answers it from counts it
    /// already has plus a two-syscall gate, so a quiet fleet costs almost
    /// nothing — and a per-row call would have put a `git` on a timer for every
    /// worktree in the sidebar, which is exactly what makes a fleet view
    /// expensive to leave open.
    func refreshChangesInbox() async {
        // Every caller's read counts against the coalescing floor, including
        // `ChangesStore.poll()`'s: a worktree with its diff open is already
        // being read on a three-second clock, and the sidebar has no reason to
        // ask again on its own tick a moment later. See `changesInboxFloor`.
        changesInboxReadAt = Date()
        let (maybe, message) = await runRaw(["changes", "inbox", "--json"], background: true)
        guard let data = maybe else {
            // Only a CONNECTED runner refusing the call proves it cannot do it.
            // A runner that is merely unreachable will answer differently once
            // it is back, and remembering "unsupported" for it would be a lie
            // that outlives the network.
            if state == .connected {
                changesSupported = false
                changesError = message
            }
            return
        }
        changesSupported = true
        changesError = nil
        guard let rows = InboxReply.rows(from: data) else { return }
        var byWorkspace: [String: InboxRow] = [:]
        // Keyed by the SHORT id, which is what `ChangesStore` and the sidebar
        // look a workspace up by. A runner new enough to send `short` is taken
        // at its word; an older one sent the short under `workspace_id` and had
        // no `short` key at all, so the fallback is not a guess.
        for r in rows { byWorkspace[r.short ?? r.workspaceId] = r }
        // Assigned only when it actually differs.
        //
        // `@Published` fires on assignment regardless of the value, and this
        // client's `objectWillChange` is wired straight into
        // `FleetStore.remerge()`, which rebuilds `fleet` unconditionally — so
        // an identical inbox assigned back over itself re-evaluates the whole
        // view tree, terminal surface included. That was affordable when this
        // ran only on a full fleet read; now that a working agent asks for it
        // every few seconds, a fleet where nobody has committed anything would
        // otherwise repaint the app on a timer to show the same numbers.
        if byWorkspace != changesInbox { changesInbox = byWorkspace }
    }

    /// When the inbox was last asked for, successfully or not.
    ///
    /// Stamped at the START of the call rather than the end, and stamped even
    /// when the call fails: a runner that refuses this in ten milliseconds must
    /// not thereby be asked ten times as often as one that answers it in a
    /// hundred. The floor is a floor on ATTEMPTS.
    private var changesInboxReadAt: Date = .distantPast
    /// Whether a coalesced read is already waiting or in flight.
    private var changesInboxArmed = false

    /// The shortest gap between two fleet-wide inbox reads.
    ///
    /// Three seconds, which is the cadence `ChangesStore.follow()` already
    /// re-reads one open worktree's change set at — and shared with it, since
    /// `refreshChangesInbox()` stamps `changesInboxReadAt` wherever it is
    /// called from, so a worktree whose diff pane is open does not pay for both
    /// clocks. Matching it also means the sidebar and the diff beside it can
    /// never be more than one tick apart, which is the same complaint the base
    /// fix was about: one worktree described two ways.
    private static let changesInboxFloor: TimeInterval = 3

    /// Re-read this runner's diff counts shortly, coalescing a burst into one
    /// call.
    ///
    /// Driven by the daemon's `change_set` event, which is the runner saying a
    /// worktree's diff actually moved — a commit, an agent's write, a checkout.
    /// This used to be driven off terminal events instead, as a stand-in for
    /// exactly that event before anything sent it, and the stand-in is gone: an
    /// agent thinking for twenty minutes produced a terminal event a second and
    /// no change to count, while an edit made in a pane that had gone quiet
    /// produced none at all.
    ///
    /// A request rather than a call, and still no timer of its own. A timer is
    /// the thing this app deliberately deleted — see `EventStream`'s own note —
    /// and every read here is a `farcooler` subprocess, over ssh for a remote
    /// runner. One armed read absorbs a whole probe pass: the daemon walks its
    /// worktrees together and can announce for several of them in the same
    /// breath, and every one of those wants the same single `changes inbox`
    /// call.
    ///
    /// A fleet where nothing is happening costs nothing at all, on both ends.
    /// The daemon spends no `git` on a worktree that nothing free says has moved
    /// — see `Watcher::probe_change_sets` — so it announces nothing, so this is
    /// never armed. A fleet where an agent is working pays one subprocess per
    /// runner per floor interval, and the daemon answers it from counts it
    /// computed once for every client rather than once per client.
    private func refreshChangesInboxSoon() {
        // One armed read absorbs every event that lands before it runs. This is
        // the whole coalescer: one probe pass on the runner can announce for
        // every worktree an agent touched, and without this each announcement
        // would be its own subprocess asking the same question.
        guard !changesInboxArmed else { return }
        // A daemon that ANSWERED and refused knows nothing about review, and
        // will refuse identically every time — see `changesSupported`. Such a
        // runner is also too old to send this event at all, so this guard is
        // belt and braces rather than the load-bearing one it was; it stays
        // because a failing subprocess on a three-second loop, each one
        // overwriting `changesError` with the same words, is not a thing to
        // leave one version away. `refresh()` still tries it unconditionally, so
        // a runner that gets upgraded is picked up on its next full read rather
        // than being written off for the lifetime of the app.
        guard changesSupported != false else { return }
        // Nothing armed for a client that was told to be quiet. `.onDisappear`
        // and a retired runner both come through `stopEvents()`, and a buffered
        // event decoded after it — which `EventStream.stop()` cannot prevent,
        // see its own note — reaches this and would otherwise launch a
        // subprocess on behalf of a window that is gone.
        guard !isStopped else { return }

        changesInboxArmed = true
        let wait = max(
            0, Self.changesInboxFloor - Date().timeIntervalSince(changesInboxReadAt))
        Task { @MainActor [weak self] in
            try? await Task.sleep(for: .seconds(wait))
            guard let self else { return }
            // Stopped WHILE waiting, which the guard above cannot see. Disarmed
            // on the way out rather than left set, so a client that comes back
            // through `startEvents()` can arm again.
            guard !self.isStopped else {
                self.changesInboxArmed = false
                return
            }
            await self.refreshChangesInbox()
            // Disarmed only once the call has RETURNED, not before it. Clearing
            // this first would let an event landing mid-call arm a second read
            // measured against a `changesInboxReadAt` the first has already
            // written — so the floor would read as satisfied and two `changes
            // inbox` subprocesses would be in flight at once for one runner,
            // racing to assign the same dictionary.
            self.changesInboxArmed = false
        }
    }

    /// One file's diff, under whichever comparison is on screen.
    ///
    /// `context` is lines of unchanged context around each hunk; zero leaves
    /// git's own default of three, and a large number is how the lines a diff
    /// omits are recovered.
    ///
    /// `commit` is the sha when the comparison is a single commit, and nil for
    /// the two that need no argument. It is a separate parameter rather than an
    /// associated value on `DiffScope` because that type is the tag of a
    /// segmented control and has to stay `CaseIterable` and `RawRepresentable`;
    /// the sha travels beside it, from `ChangesStore.selectedCommit`.
    ///
    /// Passing it here rather than having the caller assemble its own
    /// invocation keeps ONE reader of a diff: the commit path briefly went out
    /// through `changesJSON`, which republishes this client's error state as a
    /// side effect of asking for a patch.
    ///
    /// ## How an older runner is detected
    ///
    /// By what came back, not by asking what version answered. `--json` is a
    /// GLOBAL flag on `farcooler`, so every runner this app can talk to accepts
    /// it here; one whose CLI predates `c2f1117` simply ignores it on this
    /// command and prints the human patch anyway. So the decode is the probe: a
    /// payload with no `hunks` key is not this shape, and the same bytes go to
    /// `parseUnified`. That is `readCommitFiles`' rule for `changes files`,
    /// arrived at for the same reason — a runner one version behind must draw a
    /// diff, not a warning triangle.
    ///
    /// Per call, and deliberately not remembered. `changesSupported` is the
    /// cached-refusal flag in this class and its own comment says why caching
    /// one is delicate; a runner that gets upgraded mid-session starts
    /// answering in JSON on its very next file with nothing to reset.
    ///
    /// What the fallback cannot recover is the three fields that are the whole
    /// point of the JSON — the human output states them as prose, and reading
    /// prose back is the thing this change exists to stop. So an older runner
    /// still says "No textual changes" about a submodule. That is the one
    /// remaining case, it is bounded by the runner's version, and it is what
    /// updating the runner fixes.
    func changesDiff(
        workspace: String, path: String, scope: DiffScope, context: Int = 0,
        commit: String? = nil
    ) async -> FileDiff {
        var args = ["changes", "diff", workspace, path]
        // `--local`, not `--unstaged`: everything uncommitted. Asking for the
        // unstaged half alone meant a file went blank the moment it was staged,
        // which reads as the work having been undone.
        if scope == .local { args.append("--local") }
        // `--commit` and `--context` do not conflict, so expanding a collapsed
        // section works inside a commit exactly as it does on the branch.
        if let commit, scope == .commit { args += ["--commit", commit] }
        if context > 0 { args += ["--context", "\(context)"] }
        args.append("--json")
        guard let data = await run(args, background: true) else { return FileDiff() }
        if let diff = try? JSONDecoder().decode(FileDiff.self, from: data) { return diff }
        guard let text = String(data: data, encoding: .utf8) else { return FileDiff() }
        return FileDiff(lines: Self.parseUnified(text))
    }

    /// The CLI's human patch, read back into lines.
    ///
    /// The older-runner path only, since `c2f1117`. Kept rather than deleted
    /// because deleting it would make a runner one version behind show a
    /// warning triangle where a diff used to be — the same trade
    /// `parseCommitFiles` makes, and the same reason.
    ///
    /// It reads `@@` headers for the line numbers and takes the first character
    /// of each line as its kind, which is every fact this format carries. The
    /// three it cannot carry are what `FileDiff` exists for.
    static func parseUnified(_ text: String) -> [DiffComputation.Line] {
        var lines: [DiffComputation.Line] = []
        var next = 0
        var oldNo = 0
        var newNo = 0
        for raw in text.split(separator: "\n", omittingEmptySubsequences: false) {
            let line = String(raw)
            if line.hasPrefix("@@") {
                // `@@ -a,b +c,d @@`
                let parts = line.split(separator: " ")
                if parts.count >= 3 {
                    oldNo = Int(parts[1].dropFirst().split(separator: ",").first ?? "0") ?? 0
                    newNo = Int(parts[2].dropFirst().split(separator: ",").first ?? "0") ?? 0
                }
                continue
            }
            guard let first = line.first else { continue }
            let body = String(line.dropFirst())
            switch first {
            case "+":
                lines.append(
                    .init(id: next, kind: .added, oldNumber: nil, newNumber: newNo, text: body))
                next += 1
                newNo += 1
            case "-":
                lines.append(
                    .init(id: next, kind: .removed, oldNumber: oldNo, newNumber: nil, text: body))
                next += 1
                oldNo += 1
            case " ":
                lines.append(
                    .init(
                        id: next, kind: .context, oldNumber: oldNo, newNumber: newNo, text: body))
                next += 1
                oldNo += 1
                newNo += 1
            default:
                continue
            }
        }
        return lines
    }

    func changesMarkRead(workspace: String) async {
        _ = await run(["changes", "read", workspace])
    }

    /// Hand a batch of review notes to an agent pane. Nil means it went.
    ///
    /// `terminal agent-prompt`, which is the same call `AgentStream.send` makes
    /// for a typed message — so a batch of notes arrives in that agent's
    /// transcript exactly as a message typed into its composer does, and there
    /// is no second "review comment" channel for the daemon and two clients to
    /// keep in step.
    ///
    /// `runRaw` rather than `run`, for both of its differences. The failure
    /// comes BACK instead of going into `lastError`, because `lastError` draws
    /// the orange banner across the top of this pane and the outbox is already
    /// showing this failure beside the notes it kept — the same reason
    /// `runReportingError` exists for sheets. And `background: true`, because
    /// `busy` is `@Published` and toggling it re-evaluates every terminal
    /// surface in the window; the outbox has a spinner of its own that costs
    /// one popover.
    func agentPrompt(terminal: String, text: String) async -> String? {
        let (data, message) = await runRaw(
            ["terminal", "agent-prompt", terminal, text], background: true)
        if data != nil { return nil }
        return message ?? "The command didn’t finish."
    }

    private func run(_ args: [String], background: Bool = false) async -> Data? {
        let (data, message) = await runRaw(args, background: background)
        if let message { lastError = message }
        return data
    }

    /// Run a command and hand back its data or its failure message directly,
    /// rather than through `lastError`.
    ///
    /// `refresh()` used to read `lastError` back out after `run()` returned,
    /// to build `.unreachable(reason:)`. That is a race: `run()`'s failure
    /// used to write `lastError` and resume its continuation from the same
    /// hop, which closed the ordering between those two — but a SECOND,
    /// concurrent command's own `run()` call can still land its own write in
    /// between that resume and `refresh()`'s subsequent read, handing
    /// `.unreachable` a message about an unrelated command instead of its
    /// own. Returning the message removes the shared state from the middle
    /// of that read entirely: whatever this call's own result says is what
    /// `refresh()` acts on, unconditionally.
    private func runRaw(
        _ args: [String], background: Bool = false
    ) async -> (data: Data?, message: String?) {
        // A background poll must not toggle `busy`. That is a @Published change,
        // and every one of them re-evaluates the whole view tree including the
        // terminal surface, which is wasted work several times a second.
        if !background { busy = true }
        defer { if !background { busy = false } }

        guard let bin = binary else {
            let message =
                "The farcooler CLI was not found. Rebuild the app with "
                + "apps/macos/build-app.sh so it bundles one, or set FARCOOLER_BIN."
            return (nil, message)
        }

        let env = environment
        let host = cliHostArguments
        return await withCheckedContinuation { continuation in
            DispatchQueue.global(qos: .userInitiated).async {
                let process = Process()
                process.executableURL = URL(fileURLWithPath: bin)
                process.arguments = host + args
                process.environment = env

                let out = Pipe()
                let err = Pipe()
                process.standardOutput = out
                process.standardError = err

                do {
                    try process.run()
                } catch {
                    continuation.resume(returning: (nil, error.localizedDescription))
                    return
                }

                let stdout = out.fileHandleForReading.readDataToEndOfFile()
                let stderr = err.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()

                if process.terminationStatus != 0 {
                    let message =
                        String(data: stderr, encoding: .utf8)?
                        .trimmingCharacters(in: .whitespacesAndNewlines) ?? "command failed"
                    continuation.resume(returning: (nil, message))
                    return
                }

                continuation.resume(returning: (stdout, nil))
            }
        }
    }

    /// Run a command and hand back its failure instead of only banner-ing it.
    ///
    /// A sheet needs to show the reason next to the field that caused it and
    /// stay open so the user can fix it. `lastError` alone would put the
    /// message in the window behind the sheet, where nobody is looking.
    private func runReportingError(_ args: [String]) async -> String? {
        let before = lastError
        let output = await run(args)
        if output == nil {
            let message = lastError ?? "command failed"
            // Leave the banner clean: the sheet is showing this one.
            lastError = before
            return message
        }
        return nil
    }
}
