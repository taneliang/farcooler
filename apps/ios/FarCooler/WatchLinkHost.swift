import Foundation
import WatchConnectivity

/// The phone's half of the watch link: push the fleet, and perform what the
/// watch asks.
///
/// The watch holds no SSH identity and reaches no runner. Everything it wants
/// done is done HERE, through `Connection`'s core — the same calls
/// `AgentStream` makes for the phone's own screens — so a watch and a phone
/// cannot answer the same agent differently. That is not a convenience; a
/// second client that spoke to the daemon on its own would be a second place
/// for "which option did they pick" to be decided.
///
/// **This object has no connection of its own, and cannot make one.** It holds
/// a weak reference to whichever `Connection` the app is currently running, and
/// when there is none it says so in words rather than failing quietly. See
/// `perform`, and see this task's report for what that leaves unproven about a
/// phone woken from the background.
@MainActor
final class WatchLinkHost: NSObject {
    /// One per app, for the same reason the watch has one: `WCSession.default`
    /// carries a single delegate and the loser of a race for it receives
    /// nothing, with nothing logged.
    static let shared = WatchLinkHost()

    private let session = WCSession.default
    private var started = false

    /// The connection the app is currently running, if it is running one.
    ///
    /// Weak on purpose. `Connection` is a `@StateObject` owned by `FleetView`,
    /// and switching runners replaces it; a strong reference here would keep
    /// the old runner's SSH session alive forever and — far worse — would keep
    /// answering the watch through a connection to a runner the person has
    /// already navigated away from.
    private weak var connection: Connection?

    /// The last snapshot actually handed to the system, and when.
    ///
    /// Both halves are needed. See `send(snapshot:)`.
    private var lastSent: FleetSnapshot?
    private var lastSentAt = Date.distantPast

    /// Per-terminal transcripts, kept only so the SECOND question about one
    /// agent is cheap. See `pendingPermission(terminal:on:)`.
    private var replays: [String: Replay] = [:]

    private struct Replay {
        var epoch: UInt64
        var transcript: Transcript
        var askedAt: Date
    }

    private override init() { super.init() }

    /// Activate the session. Call once, as early in launch as there is a place
    /// to call it from.
    ///
    /// Early matters: iOS launches this app into the background to deliver a
    /// `sendMessage`, and a session with no delegate at that moment has nothing
    /// to hand the message to. That is why the call site is
    /// `PushDelegate.application(_:didFinishLaunchingWithOptions:)` and not a
    /// SwiftUI `.task` — a background launch may never build a scene at all, so
    /// a `.task` on a view is a hook that does not run in exactly the case this
    /// feature exists for.
    func start() {
        guard WCSession.isSupported(), !started else { return }
        started = true
        session.delegate = self
        session.activate()
    }

    /// Point the link at the connection the app is now running.
    ///
    /// Called from `Connection.start`, before it has connected. Deliberately
    /// before: what the watch needs is a way to reach the runner the person is
    /// actually looking at, and `perform` checks the phase itself. Registering
    /// only on success would leave the watch with no connection at all during
    /// the seconds a reconnect takes, and answer "open the app" to somebody who
    /// has it open.
    func adopt(_ connection: Connection) {
        self.connection = connection
        // A different runner is a different fleet. Its transcripts are keyed on
        // terminal ids that mean nothing over there, and answering a permission
        // against the wrong runner is the one mistake this whole seam exists
        // to make impossible.
        replays.removeAll()
    }

    // MARK: - Pushing the fleet

    /// Hand the watch the fleet the phone just polled.
    ///
    /// Sent as an application context, which is latest-wins and coalesced by the
    /// system — exactly a snapshot's semantics, and the reason this can be
    /// called on every poll without building a queue of stale fleets.
    ///
    /// Two guards on top of that, and neither is an optimization:
    ///
    ///   - **Unchanged agents are not resent.** `capturedAt` moves on every
    ///     poll, so `FleetSnapshot`'s own `==` is true perhaps never; comparing
    ///     the agents is what makes "nothing happened" recognizable. This buys
    ///     an IDLE fleet only, and deliberately so: `line` and `feed` are part
    ///     of `Agent`'s equality and churn on nearly every poll while an agent
    ///     is working, so a working fleet still pushes at the full three-second
    ///     poll rate. That is the right way round — it is also when somebody is
    ///     watching — and `updateApplicationContext` coalesces what the link
    ///     cannot carry. What this guard removes is the case with nothing to
    ///     show for it: a fleet where nothing is happening, paying for a
    ///     Bluetooth write twenty times a minute.
    ///   - **…but at least every 30 seconds anyway.** The watch says how old its
    ///     fleet is, and it reads that off `capturedAt`. Sending only on change
    ///     would make a quiet fleet look abandoned: ten minutes of nothing
    ///     happening would render as a ten-minute-old snapshot, which is the
    ///     watch saying "I have not heard from your phone" about a phone that
    ///     has been polling happily the whole time.
    func send(snapshot: FleetSnapshot) {
        guard session.activationState == .activated else { return }
        // No watch paired, or the app not installed on it. Not an error and not
        // worth a log line — most people running this app own no Apple Watch.
        guard session.isPaired, session.isWatchAppInstalled else { return }

        // The agents AND the review count, because either can move without the
        // other. A worktree gaining an unreviewed diff changes no agent — the
        // count is per worktree and an agent is per terminal — so comparing
        // agents alone left the wrist up to thirty seconds behind on exactly
        // the number the glance was added to show. The thirty-second ceiling
        // below still bounds it; this is about not spending that ceiling on a
        // change we already know about.
        let changed =
            snapshot.agents != lastSent?.agents
            || snapshot.reviewsWaiting != lastSent?.reviewsWaiting
        guard changed || Date().timeIntervalSince(lastSentAt) >= Self.refreshInterval else {
            return
        }
        guard let data = try? JSONEncoder().encode(snapshot) else { return }
        // `try?` because the only errors are "not activated" and "not a
        // property-list value", both of which are already excluded above, and
        // there is no user-facing consequence to report: the next poll sends
        // the next snapshot thirty seconds from now at the latest.
        try? session.updateApplicationContext([Self.snapshotKey: data])
        lastSent = snapshot
        lastSentAt = Date()
    }

    private static let refreshInterval: TimeInterval = 30

    /// Spelled the same as `WatchLinkClient.snapshotKey`, and it has to be.
    ///
    /// The two are in separate binaries — this one cannot see that one — which
    /// is why `WatchLink.swift` exists for every OTHER string that crosses this
    /// seam. This single key could not join it without putting a transport
    /// detail into the shared vocabulary. A mismatch is silent in the way this
    /// whole file warns about: the context arrives, the watch finds nothing
    /// under the key it looked for, and shows an empty fleet forever.
    private static let snapshotKey = "snapshot"

    // MARK: - Performing what the watch asks

    /// The three requests, each performed through the code path the phone's own
    /// UI uses.
    private func perform(_ request: WatchRequest) async -> WatchReply {
        guard let connection else {
            // No `FleetView` on screen means no `Connection` object at all —
            // there is no app-wide one to fall back to. Honest, and
            // actionable. `appName`, not a literal: a canary build is named
            // "FC Canary" and telling somebody running it to open "Far
            // Cooler" sends them looking for an app that is not on their
            // phone, the same mistake already fixed once in the Live
            // Activity and once in the widget.
            return .failed("Open \(appName) on your iPhone, then try again.")
        }
        guard await ready(connection) else {
            // Named apart because it is the one unreachable state a person can
            // do something about, and "can’t reach that runner" would send them
            // to check their Wi-Fi instead of to the fingerprint waiting on
            // their phone.
            if case .needsApproval = connection.phase {
                return .failed("Approve this runner on your iPhone first.")
            }
            return .failed("Your iPhone can’t reach that runner right now.")
        }

        switch request {
        case let .prompt(terminal, text):
            // `AgentStream.send`'s call, without the stream. There is no
            // transcript to echo into here and no view to update, so
            // constructing one would mean a second poller racing whichever
            // screen the phone has open against the same pane.
            return await call(
                "terminal.agent_prompt", ["terminal": terminal, "text": text],
                on: connection, doing: "send that")

        case let .answer(terminal, request, option):
            let reply = await call(
                "terminal.agent_answer",
                ["terminal": terminal, "requestId": request, "optionId": option],
                on: connection, doing: "answer that")
            // Cleared on SUCCESS only, and the ordering is the whole point.
            //
            // It has to be cleared at all for the reason
            // `Transcript.clearPendingPermission` documents: the agent resumes
            // without acknowledging what it was blocked on, so nothing ever
            // arrives to retire this and the next question about this terminal
            // would re-offer a permission already answered.
            //
            // But clearing it BEFORE the call — which this did — makes a failed
            // answer permanent. Tap Allow, the link drops, the watch says so,
            // and the retry asks again from the cursor this cache already
            // reached: a delta with no `Permission` event in it, because that
            // event is behind the cursor. The watch would then say "nothing to
            // answer" about an agent that is still blocked, for as long as this
            // process lives. A transient stream would have self-healed by
            // replaying from zero; a cache does not, so it must not forget
            // anything it has not established is gone.
            if case .sent = reply { replays[terminal]?.transcript.clearPendingPermission() }
            return reply

        case let .pendingPermission(terminal):
            return await pendingPermission(terminal: terminal, on: connection)
        }
    }

    /// One core call, reported as `sent` or as a sentence.
    private func call(
        _ method: String, _ args: [String: Any], on connection: Connection, doing what: String
    ) async -> WatchReply {
        do {
            _ = try await withTimeout(seconds: Self.actionBudget) {
                try await connection.core.call(method, args)
            }
            // "The phone handed it on", not "the agent acted on it". See
            // `WatchReply.sent`, which is careful about exactly this.
            return .sent
        } catch {
            return .failed(Self.reason(error, doing: what))
        }
    }

    /// What, if anything, this agent is blocked on.
    ///
    /// A permission's id and its options exist only in the agent's event
    /// stream — the fleet snapshot carries a headline, not a request id — so
    /// this is the one request that cannot be answered from what the phone
    /// already holds. There is no cheaper source: the daemon exposes no "what
    /// is pending" call, and `blocked_question` in `watch.rs` is a line scraped
    /// off the screen with no id and no options attached to it.
    ///
    /// So it replays the stream. Three things make that affordable:
    ///
    ///   - **One round trip, not a poll loop.** `terminal.agent_subscribe`
    ///     answers with the daemon's whole retained window in a single reply —
    ///     bounded at `TRANSCRIPT_LIMIT`, 4096 events, in
    ///     `agent_supervisor.rs`. `AgentStream` calls the same method every
    ///     700ms because it is watching a live conversation; this is asking one
    ///     question once, so it asks once.
    ///   - **The second question is a delta.** The folded transcript is kept per
    ///     terminal, so a later ask sends the cursor it reached and gets back
    ///     only what happened since. Checking a blocked agent twice costs one
    ///     replay, not two.
    ///   - **A timeout, and the timeout answers `nil`.** See `replayBudget`.
    ///
    /// `nil` is "nothing pending", which the vocabulary already means and the
    /// watch already renders as such — an agent can be blocked on a trust gate
    /// or a plain question, and reporting that as a failure would put an error
    /// in front of somebody when nothing went wrong.
    private func pendingPermission(terminal: String, on connection: Connection) async -> WatchReply {
        var replay = replays[terminal] ?? Replay(epoch: 0, transcript: Transcript(), askedAt: Date())
        // Read out before the closure captures anything. `replay` is a `var`
        // this function goes on to mutate, and a concurrently-executing closure
        // that read it directly would be reading a value someone else is
        // writing — an error outright under the Swift 6 language mode.
        let fromSeq = Int(clamping: replay.transcript.cursor)
        let epoch = Int(clamping: replay.epoch)

        let data: Data
        do {
            data = try await withTimeout(seconds: Self.replayBudget) {
                try await connection.core.call(
                    "terminal.agent_subscribe",
                    ["terminal": terminal, "fromSeq": fromSeq, "epoch": epoch])
            }
        } catch is Timeout {
            // NOT `.permission(nil)`. Twenty lines below, an unreadable reply
            // is refused that answer because it "would tell them their agent is
            // not waiting when it may well be" — and a timeout on a slow link
            // says precisely the same untrue thing. The asking is not idle
            // either: the watch only asks because the snapshot said `blocked`,
            // so an agent that IS waiting is the likely case here rather than
            // the edge one. `.permission(nil)` is kept for what it means — the
            // replay finished and there was nothing pending.
            return .failed("Couldn’t read that agent in time. Open it on your iPhone.")
        } catch {
            return .failed(Self.reason(error, doing: "check that agent"))
        }

        guard let batch = try? JSONDecoder().decode(AgentStream.Batch.self, from: data) else {
            // Unreadable is not "nothing pending" — but it is not something a
            // watch screen can act on either, and there is no third answer in
            // the vocabulary. Said as a failure so it reaches somebody, rather
            // than as `nil`, which would tell them their agent is not waiting
            // when it may well be.
            return .failed("Couldn’t read that agent’s conversation.")
        }

        // The same epoch rule `AgentStream.pump` follows: a different epoch
        // means the stream restarted and every number this holds counts
        // positions in a conversation that no longer exists. The daemon returns
        // the whole window in that case, so the transcript is replaced rather
        // than appended to.
        if batch.epoch != replay.epoch {
            replay.epoch = batch.epoch
            replay.transcript.resetForNewEpoch()
        }
        replay.transcript.apply(
            batch.events.map { frame in
                // Exactly `AgentStream`'s fallback: an unreadable frame becomes
                // a gap rather than vanishing. It costs nothing here — a gap
                // holds no permission — and dropping it would leave the cursor
                // pointing past an event this transcript never folded.
                Sequenced(
                    seq: frame.seq,
                    event: (try? AgentEvent.decode(from: frame.payloadJson)) ?? .gap(.unparsed))
            })
        replay.askedAt = Date()
        replays[terminal] = replay
        prune()

        guard let pending = replay.transcript.pendingPermission else { return .permission(nil) }
        // Field for field, which is why `WatchPermission` was written to match
        // `PendingPermission` exactly: a copy has nothing to decide.
        return .permission(
            WatchPermission(
                id: pending.id,
                toolCall: pending.toolCall,
                options: pending.options.map {
                    WatchPermissionOption(id: $0.id, name: $0.name, kind: $0.kind)
                }))
    }

    /// Keep the cache to the handful of agents somebody actually answers from
    /// their wrist.
    ///
    /// Each entry is one folded transcript, and it is NOT bounded by the
    /// daemon's 4096-event window: the first ask brings back at most that
    /// much, but every ask after it appends a delta and nothing here ever
    /// trims. The real ceiling is the whole conversation, for as long as this
    /// process lives — the daemon's window bounds one REPLY, not the
    /// accumulation of them.
    ///
    /// So the bound that exists is this one: eight terminals, evicting the
    /// least recently asked. Not a leak — the count is fixed and `adopt` empties
    /// the whole cache — but a phone left running for a week would otherwise
    /// hold one growing transcript per agent it was ever asked about, and none
    /// of them would ever be needed again.
    private func prune() {
        guard replays.count > Self.replayCacheLimit else { return }
        let oldest = replays.min { $0.value.askedAt < $1.value.askedAt }
        if let oldest { replays.removeValue(forKey: oldest.key) }
    }

    private static let replayCacheLimit = 8

    // MARK: - Reaching the runner

    /// Whether this connection can carry a call right now, waiting a little if
    /// it is on its way back.
    ///
    /// The case this exists for: the phone was suspended in a pocket, iOS woke
    /// it to deliver the watch's message, and its SSH session died some time
    /// during the suspension. `Connection` recovers from that on its own — but
    /// on its own schedule, driven by a poller that is not running yet.
    /// `reconnectNow` is the same escape hatch the UI offers when you can see
    /// it is stuck, used here for the same reason: something outside the
    /// backoff timer knows a connection is wanted this second.
    private func ready(_ connection: Connection) async -> Bool {
        if connection.phase == .connected { return true }
        // Not from `.failed` or `.needsApproval`. Those are waiting on a person
        // — a host key to trust, a key to authorize — and no amount of retrying
        // from a wrist gets past them.
        guard connection.phase == .connecting || isReconnecting(connection.phase) else {
            return false
        }
        // Hurried along only when it is WAITING. `.reconnecting` can be sitting
        // out a thirty second backoff — longer than the whole budget here — and
        // this is exactly the case `reconnectNow` exists for: something outside
        // the timer knows a connection is wanted now. `.connecting` is already
        // trying, and interrupting it would throw away a handshake in progress
        // and start the slowest part over.
        if isReconnecting(connection.phase) { connection.reconnectNow() }
        let deadline = Date().addingTimeInterval(Self.connectBudget)
        while Date() < deadline {
            if connection.phase == .connected { return true }
            if case .failed = connection.phase { return false }
            try? await Task.sleep(for: .milliseconds(200))
        }
        return false
    }

    private func isReconnecting(_ phase: Connection.Phase) -> Bool {
        if case .reconnecting = phase { return true }
        return false
    }

    /// How long each stage may take, in seconds.
    ///
    /// All three are budgets against ONE clock: `sendMessage`'s reply handler.
    /// The watch is waiting on a spinner the whole time, and a reply that comes
    /// too late is not a slow success — WatchConnectivity gives up and the watch
    /// is told the phone did not answer. So the phone must fail on purpose,
    /// early, with something readable, rather than be timed out with something
    /// generic.
    ///
    /// `connectBudget` is the longest because reviving a dead SSH session is
    /// the slowest thing here and the only one with nothing to show for a
    /// partial result. `actionBudget` covers a prompt or an answer, which are
    /// one small call each. `replayBudget` covers a transcript replay, whose
    /// size is not knowable in advance; on timeout it fails in words that send
    /// the person to their phone, rather than leaving them looking at a spinner
    /// that will not finish or telling them there is nothing to answer.
    private static let connectBudget: TimeInterval = 8
    private static let actionBudget: TimeInterval = 8
    private static let replayBudget: TimeInterval = 10

    private struct Timeout: Error {}

    /// Stop waiting after `seconds`.
    ///
    /// It does not stop the WORK: a core call is a ticket the Rust side
    /// resolves whenever the network gets round to it, and cancelling the task
    /// that awaits it does not reach across the FFI boundary. What this buys is
    /// an answer for the watch within the window it has — the call may well
    /// land a moment later, into nothing.
    private func withTimeout<T: Sendable>(
        seconds: TimeInterval, _ work: @escaping @Sendable () async throws -> T
    ) async throws -> T {
        try await withThrowingTaskGroup(of: T.self) { group in
            group.addTask { try await work() }
            group.addTask {
                try await Task.sleep(for: .seconds(seconds))
                throw Timeout()
            }
            defer { group.cancelAll() }
            guard let first = try await group.next() else { throw Timeout() }
            return first
        }
    }

    /// The core's failure, as a sentence for a watch face.
    ///
    /// Never the raw string from Rust. `ClientCore.CoreError` carries the
    /// daemon's `Display` output, which is written for a terminal and is
    /// routinely longer than a watch screen — and "Stopped waiting: ssh channel
    /// closed" in place of an answer is not a message, it is a leak.
    ///
    /// The `Timeout` sentence is careful about one thing in particular: it does
    /// NOT say the work did not happen, because this side cannot know that.
    /// `withTimeout`, twenty lines up, says so itself — it stops waiting, it
    /// does not stop the call, and "the call may well land a moment later, into
    /// nothing." A permission answered **Deny** that timed out on the way back
    /// is a Deny that was carried out, and telling the watch nothing was sent
    /// is how somebody taps **Allow** next and gets the opposite of what they
    /// chose. So it says what is known — no answer — and what to do about it.
    private static func reason(_ error: Error, doing what: String) -> String {
        if error is Timeout {
            return "Your iPhone took too long. It may still have gone through — "
                + "check before trying again."
        }
        let text = error.localizedDescription.lowercased()
        if text.contains("not found") { return "That agent isn’t running anymore." }
        return "Couldn’t \(what). Your iPhone lost touch with the runner."
    }
}

/// The delegate half, `nonisolated` because WatchConnectivity calls back on its
/// own queue.
///
/// `replyHandler` is the whole contract of `sendMessage`: the watch is holding
/// a continuation open until it runs. Every path through here calls it exactly
/// once — including the paths where the request could not be read at all, since
/// a handler that is never called leaves the watch waiting for a timeout it
/// will report as "your iPhone didn’t answer".
extension WatchLinkHost: WCSessionDelegate {
    nonisolated func session(
        _ session: WCSession,
        activationDidCompleteWith activationState: WCSessionActivationState,
        error: Error?
    ) {}

    /// Required on iOS. The watch is being unpaired or switched; the system
    /// wants the session reactivated for the new one.
    nonisolated func sessionDidBecomeInactive(_ session: WCSession) {}

    nonisolated func sessionDidDeactivate(_ session: WCSession) {
        // Reactivating is what makes a switched watch work. Without it this
        // session stays deactivated for the life of the process and the new
        // watch receives nothing, which reads as "the watch app is broken".
        session.activate()
    }

    nonisolated func session(
        _ session: WCSession,
        didReceiveMessage message: [String: Any],
        replyHandler: @escaping ([String: Any]) -> Void
    ) {
        guard let request = WatchRequest(dictionary: message) else {
            // A word this build has not learned — a newer watch against an older
            // phone. Answered rather than dropped, and answered as a failure
            // rather than as a success for something that did not happen.
            // `appName`, not a literal — see `perform`'s `.failed` above.
            replyHandler(WatchReply.failed("Update \(appName) on your iPhone.").dictionary)
            return
        }
        Task { @MainActor in
            let reply = await self.perform(request)
            replyHandler(reply.dictionary)
        }
    }
}

/// This build's app name, for the two watch-link sentences that have to say
/// it.
///
/// `CFBundleDisplayName`, which `generate-project.py` stamps into this target
/// from `version.sh app-name-short`. A literal would tell somebody running the
/// canary to open "Far Cooler", which is either a different app on their
/// phone or no app at all — the mistake a hardcoded name already made once in
/// the Live Activity and once in the widget, both of which read their own
/// bundle for the same reason this one now does.
///
/// Not `@MainActor`, matching `appName` in `FleetWidget.swift` and
/// `WatchFleetWidget.swift`: `Bundle.main` needs no actor, and one of this
/// property's two call sites is `WatchLinkHost`'s `nonisolated` delegate
/// method, which cannot `await` its way onto the main actor just to read a
/// string.
private var appName: String {
    Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "Far Cooler"
}
