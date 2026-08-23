import ActivityKit
import Foundation
import WatchConnectivity

/// The phone's half of the watch link: push the fleet, and perform what the
/// watch asks — and, since the Live Activity grew buttons, what a lock screen
/// card asks too.
///
/// The watch holds no SSH identity and reaches no runner. Everything it wants
/// done is done HERE, through `Connection`'s core — the same calls
/// `AgentStream` makes for the phone's own screens — so a watch and a phone
/// cannot answer the same agent differently. That is not a convenience; a
/// second client that spoke to the daemon on its own would be a second place
/// for "which option did they pick" to be decided.
///
/// **The widget extension is the second such surface, and it arrives here for
/// the same reason.** `AnswerPermissionIntent` runs in the app's process and
/// hands itself to `answerFromGlance` below, which reaches the runner through
/// the one `Connection` this object already holds. The name on the front of
/// this file is now narrower than what it does; renaming it would move the
/// connection registration in `Connection.start`, the delegate that
/// `PushDelegate` activates at launch and the replay cache all at once, which
/// is a large diff to buy a better noun. What matters is that there is still
/// exactly one place where a surface without a connection gets one.
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
    /// agent is cheap — and there are now two questions that read one, so the
    /// second is routinely asked. See `replay(terminal:on:within:)`.
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
        //
        // `agentsSayTheSame(as:)` rather than `!=` on the arrays, and the
        // difference is one field: `observedAt` moves on every poll for every
        // agent, by design, so plain inequality is true every three seconds
        // forever and this guard would stop guarding anything. The question
        // here has always been "would the watch draw something different", and
        // when we last heard about an agent is not something it draws.
        //
        // Nothing is lost by leaving it out. The thirty-second floor below
        // resends regardless, so the wrist's `observedAt` is never more than
        // thirty seconds behind this phone's — against a `staleAfter` of an
        // hour.
        let changed =
            !snapshot.agentsSayTheSame(as: lastSent)
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

    /// The four requests, each performed through the code path the phone's own
    /// UI uses.
    ///
    /// Its sentences are read on a WRIST and still name the device by
    /// `DeviceKind` rather than by a literal. On this path the two are the same
    /// answer — `WCSession.isSupported()` is false on iPad, so `start()` never
    /// activates a session there and nothing below ever runs on one — and
    /// resolving it keeps ONE spelling of the device across the watch link and
    /// the lock screen card, which `answerFromGlance` also feeds from here.
    private func perform(_ request: WatchRequest) async -> WatchReply {
        guard let connection else {
            // No `FleetView` on screen means no `Connection` object at all —
            // there is no app-wide one to fall back to. Honest, and
            // actionable. `appName`, not a literal: a canary build is named
            // "FC Canary" and telling somebody running it to open "Far
            // Cooler" sends them looking for an app that is not on their
            // phone, the same mistake already fixed once in the Live
            // Activity and once in the widget.
            return .failed("Open \(appName) on your \(DeviceKind.current), then try again.")
        }
        guard await ready(connection, within: Self.connectBudget) else {
            // Named apart because it is the one unreachable state a person can
            // do something about, and "can’t reach that runner" would send them
            // to check their Wi-Fi instead of to the fingerprint waiting on
            // their phone.
            if case .needsApproval = connection.phase {
                return .failed("Approve this runner on your \(DeviceKind.current) first.")
            }
            return .failed("Your \(DeviceKind.current) can’t reach that runner right now.")
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
            // The card and the wrist look at the same agent, so an answer sent
            // from one has to take the other's buttons down. Only on success,
            // for the reason the replay cache below is cleared only on success:
            // a failed answer leaves the agent waiting, and a card that quietly
            // stopped offering the answer would be the surface hiding the one
            // thing still worth doing.
            if case .sent = reply {
                GlancePermissionStore.update { $0.clearingPermission(for: terminal) }
            }
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
            let reply = await pendingPermission(
                terminal: terminal, on: connection, within: Self.replayBudget)
            // An OBSERVATION, so it is written down where a surface with no
            // connection can use it.
            //
            // Here rather than inside `pendingPermission` deliberately:
            // `answerFromGlance` calls that same method to check an id it is
            // about to send, and a write on that path would file a fresh record
            // in the middle of an answer that has already been claimed against
            // the previous one. Reading is not observing; being ASKED what an
            // agent is waiting on is.
            if case let .permission(found) = reply { Self.record(found, for: terminal) }
            return reply

        case let .transcript(terminal):
            return await transcript(
                terminal: terminal, on: connection, within: Self.replayBudget)
        }
    }

    /// File what an agent turned out to be waiting on, for the surfaces that
    /// cannot ask.
    ///
    /// `nil` is written as emphatically as a permission is: the caller
    /// established that this agent is waiting on nothing, and a card left
    /// offering yesterday's answers is the failure `GlancePermissions` was
    /// written to prevent.
    static func record(_ permission: WatchPermission?, for terminal: String) {
        let observed = permission.map { pending in
            GlancePermission(
                terminal: terminal,
                request: pending.id,
                // Field for field, the same copy `WatchPermission` itself is of
                // `PendingPermission`. Three spellings of three fields, and
                // each crossing is a boundary between two binaries — see
                // `GlancePermissionOption`.
                options: pending.options.map {
                    GlancePermissionOption(id: $0.id, name: $0.name, kind: $0.kind)
                },
                observedAt: Date())
        }
        GlancePermissionStore.update { $0.recording(observed, for: terminal) }
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
    /// stream, so this cannot be answered from anything the phone already
    /// holds; `replay` below is what goes and gets it, and carries the reasoning
    /// about what that costs.
    ///
    /// `nil` is "nothing pending", which the vocabulary already means and the
    /// watch already renders as such — an agent can be blocked on a trust gate
    /// or a plain question, and reporting that as a failure would put an error
    /// in front of somebody when nothing went wrong. It is emphatically NOT
    /// what a failed replay answers: see `replay`'s two `catch`es.
    private func pendingPermission(
        terminal: String, on connection: Connection, within budget: TimeInterval
    ) async -> WatchReply {
        switch await replay(terminal: terminal, on: connection, within: budget) {
        case let .failed(reason):
            return .failed(reason)
        case let .folded(transcript, _):
            guard let pending = transcript.pendingPermission else { return .permission(nil) }
            // Field for field, which is why `WatchPermission` was written to
            // match `PendingPermission` exactly: a copy has nothing to decide.
            return .permission(
                WatchPermission(
                    id: pending.id,
                    toolCall: pending.toolCall,
                    options: pending.options.map {
                        WatchPermissionOption(id: $0.id, name: $0.name, kind: $0.kind)
                    }))
        }
    }

    /// What this agent has SAID, cut to what the link will carry.
    ///
    /// The same replay `pendingPermission` runs, read for a different fact —
    /// which is the whole reason that function was split. A watch asking "what
    /// did it say" and a watch asking "what is it waiting on" are one round
    /// trip to the runner either way, and the second of them is free because
    /// the folded transcript is already in hand.
    ///
    /// **What is kept: the messages, and only the top-level ones.** The
    /// transcript's own vocabulary is `message`, `tool`, `subagent` and `gap`,
    /// and three of those four are dropped here:
    ///
    ///   - `tool` rows are `Read crates/core/src/feed.rs` and diffs. The
    ///     agent's ACTIVITY, which is exactly what `FleetSnapshot.Agent.feed`
    ///     already carries onto the wrist, already truncated by the host for
    ///     the purpose. Sending them again as prose would spend the budget
    ///     re-answering a question the fleet row answered.
    ///   - `subagent` blocks nest a whole second conversation. A dispatch and
    ///     its children on a 45mm screen is a tree, and the person opening this
    ///     wants a sentence.
    ///   - `gap` says something was lost. It is not words, so it is not an
    ///     entry — but it does mean this is not the whole conversation, so it
    ///     is folded into `complete` below rather than discarded.
    ///
    /// `Role.thought` goes too, and it is the only judgement call here. Thinking
    /// is long, it is not addressed to anybody, and it is not what the agent
    /// answered — "an agent completed answering my question and I want to see
    /// what it said" is the sentence this screen exists for, and a wrist full
    /// of reasoning would bury the answer under it.
    ///
    /// `Role.user` stays, which is the other half of that sentence. An answer
    /// with no question above it is a paragraph nobody can place, and a prompt
    /// is short — the phone's own composer sends one line, and dictation from
    /// this very watch sends less.
    private func transcript(
        terminal: String, on connection: Connection, within budget: TimeInterval
    ) async -> WatchReply {
        switch await replay(terminal: terminal, on: connection, within: budget) {
        case let .failed(reason):
            return .failed(reason)
        case let .folded(transcript, _):
            var entries: [WatchTranscriptEntry] = []
            var whole = true
            for row in transcript.rows {
                switch row.kind {
                case let .message(role, text, parent):
                    // `parent` non-nil is an orphaned subagent message — a
                    // child whose block never arrived. `Transcript` keeps it at
                    // the top level rather than losing it, and is explicit that
                    // merging it with the agent's own words would be a
                    // mis-attribution. So it is dropped here for the same
                    // reason the blocks are, and counted against `whole`.
                    guard parent == nil else {
                        whole = false
                        continue
                    }
                    guard role != .thought else { continue }
                    let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
                    guard !trimmed.isEmpty else { continue }
                    entries.append(WatchTranscriptEntry(role: role.rawValue, text: trimmed))
                case let .gap(reason):
                    // Words this phone never received. Not an entry, but the
                    // one thing that makes "these are all the words there were"
                    // untrue without anything here having dropped a thing.
                    //
                    // **Except `loadEmpty`, which is not a loss and is the
                    // ordinary case.** It means nothing was ever recorded for
                    // this session — a claude or codex terminal opened as a
                    // chat before its first turn — so an empty fold IS the
                    // whole conversation. Counting it would put "there's more
                    // on your iPhone" under the words of every agent that has
                    // only just started, which is the reading this screen was
                    // built to stop somebody making.
                    //
                    // The other four do count, `loadUnsupported` and
                    // `loadFailed` included, even though the phone has no more
                    // of those words than the watch does. What is on the phone
                    // in that case is the gap ITSELF, rendered and explained —
                    // and somebody who came looking for words that are not
                    // there is better served by finding out why than by a watch
                    // that quietly presents a fragment as the whole thing.
                    if reason != .loadEmpty { whole = false }
                case .tool, .subagent:
                    continue
                }
            }
            return .transcript(WatchTranscript.fitting(entries, whole: whole))
        }
    }

    /// The outcome of one replay: a folded conversation, or a sentence.
    ///
    /// A `Result` in all but name, spelled out because the failure side is not
    /// an `Error` — it is already the sentence a person reads, produced by
    /// `reason(_:doing:)` or written out beside the timeout it describes.
    private enum Replayed {
        /// The transcript, and the epoch it belongs to. The epoch is carried
        /// for the same reason `Replay` stores it: two folds of two different
        /// streams are not comparable, and a caller that cached anything off
        /// one of them needs to know which.
        case folded(Transcript, epoch: UInt64)
        case failed(String)
    }

    /// Bring this terminal's conversation up to date, and hand it over.
    ///
    /// This is the body `pendingPermission` used to be, and every word of its
    /// reasoning still applies — one round trip rather than a poll loop, a
    /// delta on the second ask, and a timeout that fails in words rather than
    /// answering "nothing".
    ///
    /// A permission's id and its options exist only in the agent's event
    /// stream — the fleet snapshot carries a headline, not a request id — and
    /// so does everything the agent said. There is no cheaper source for
    /// either: the daemon exposes no "what is pending" call, and
    /// `blocked_question` in `watch.rs` is a line scraped off the screen with no
    /// id and no options attached to it.
    ///
    /// Three things make the replay affordable:
    ///
    ///   - **One round trip, not a poll loop.** `terminal.agent_subscribe`
    ///     answers with the daemon's whole retained window in a single reply —
    ///     bounded at `TRANSCRIPT_LIMIT`, 4096 events, in
    ///     `agent_supervisor.rs`. `AgentStream` calls the same method every
    ///     700ms because it is watching a live conversation; this is asking one
    ///     question once, so it asks once.
    ///   - **The second question is a delta.** The folded transcript is kept per
    ///     terminal, so a later ask sends the cursor it reached and gets back
    ///     only what happened since. Reading an agent twice costs one replay,
    ///     not two — and asking what it said and then what it is waiting on
    ///     costs one between them.
    ///   - **A timeout, and the timeout does not lie.** See `replayBudget`.
    private func replay(
        terminal: String, on connection: Connection, within budget: TimeInterval
    ) async -> Replayed {
        var replay = replays[terminal] ?? Replay(epoch: 0, transcript: Transcript(), askedAt: Date())
        // Read out before the closure captures anything. `replay` is a `var`
        // this function goes on to mutate, and a concurrently-executing closure
        // that read it directly would be reading a value someone else is
        // writing — an error outright under the Swift 6 language mode.
        let fromSeq = Int(clamping: replay.transcript.cursor)
        let epoch = Int(clamping: replay.epoch)

        let data: Data
        do {
            data = try await withTimeout(seconds: budget) {
                try await connection.core.call(
                    "terminal.agent_subscribe",
                    ["terminal": terminal, "fromSeq": fromSeq, "epoch": epoch])
            }
        } catch is Timeout {
            // NOT an empty answer. `.permission(nil)` means the replay finished
            // and there was nothing pending, and an empty `WatchTranscript`
            // means the agent has said nothing — both are established facts,
            // and a timeout establishes neither. The asking is not idle either:
            // the watch asks about a permission because the snapshot said
            // `blocked`, and asks for a transcript because somebody wants to
            // read one, so there is something here in the likely case rather
            // than the edge one.
            return .failed(
                "Couldn’t read that agent in time. Open it on your \(DeviceKind.current).")
        } catch {
            return .failed(Self.reason(error, doing: "check that agent"))
        }

        guard let batch = try? JSONDecoder().decode(AgentStream.Batch.self, from: data) else {
            // Unreadable is not an empty answer either, for the reason above,
            // and there is no third case in the vocabulary. Said as a failure
            // so it reaches somebody rather than as an emptiness that reads
            // like a fact.
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

        return .folded(replay.transcript, epoch: replay.epoch)
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

    // MARK: - Answering from a glance surface

    /// Let `AnswerPermissionIntent` reach this object.
    ///
    /// Called from `PushDelegate.application(_:didFinishLaunchingWithOptions:)`
    /// beside `start()`, and the call site matters for the same reason it
    /// matters there: iOS launches this app into the background to perform an
    /// app intent, a background launch may never build a scene, and a hook
    /// installed from a SwiftUI `.task` would therefore be missing in exactly
    /// the case a lock screen button exists for.
    ///
    /// Separate from `start()` rather than folded into it, because `start()`
    /// returns early when `WCSession.isSupported()` is false — which it is on
    /// an iPad — and a card's buttons have nothing to do with whether this
    /// device can pair a watch.
    func acceptAnswersFromGlances() {
        AnswerPermissionDelivery.handler = { intent in
            await WatchLinkHost.shared.answerFromGlance(intent)
        }
    }

    /// Answer one permission on behalf of a surface that cannot reach a runner,
    /// and leave behind an honest account of what happened.
    ///
    /// Five things happen in order, and the ORDER is the design:
    ///
    ///   1. **Claim, or stop.** `GlancePermissions.claiming` refuses when this
    ///      phone already has an answer standing against this request id, which
    ///      is what makes a second tap impossible rather than merely unlikely.
    ///      It has to be enforced here because nothing downstream will:
    ///      `terminal.agent_answer` posts a message and returns without
    ///      waiting, and `AgentEvent::Resolved` — the one event that could
    ///      retire a permission — is emitted by nothing in the tree. A widget
    ///      also has no in-flight state of its own; the claim IS the disabled
    ///      state that `PermissionView` gets from `@State`.
    ///   2. **A connection, or a sentence.** Both refusals are the ones the
    ///      watch already gets, word for word, because they are the same two
    ///      conditions and two wordings for one condition would read as two
    ///      problems.
    ///   3. **Verify before writing.** The options came out of a file whose age
    ///      is not knowable from here, so the id is checked against the agent's
    ///      own stream before anything is sent. This is the only place a stale
    ///      answer can be caught at all — see the note on step 1 — and catching
    ///      it is what makes "answering a permission somebody already answered
    ///      fails visibly" true rather than aspirational.
    ///   4. **One attempt.** Never a retry. ACP's `session/prompt` is sent with
    ///      `request_no_wait` and nothing acknowledges an answer, so a retry is
    ///      a second answer to a live agent, not a second chance at the first.
    ///   5. **Say what is known.** `sent`, `unsure` and `nothingSent` are kept
    ///      apart with the care `WatchLinkClient` spells out, because the
    ///      failure they exist to prevent is the one this surface cannot take
    ///      back: a reject that landed, reported as unsent, followed by a tap
    ///      on the option that allows.
    ///
    /// **What is deliberately NOT done here: verification is refused, not
    /// skipped.** If the agent's stream cannot be read, this does not send
    /// anyway on the theory that the permission is probably still open. A card
    /// that says "couldn't check that agent" and leaves the buttons off is a
    /// worse experience and a correct one; a duplicate answer to a live agent
    /// is neither.
    private func answerFromGlance(_ intent: AnswerPermissionIntent) async {
        let terminal = intent.terminal
        let request = intent.request

        guard
            let claimed = GlancePermissionStore.read().claiming(
                terminal: terminal, request: request, option: intent.option,
                optionName: intent.optionName, at: Date())
        else {
            // Already answered from this phone. Nothing is written, because the
            // record that refused this tap is the one already on the card and
            // overwriting it would replace the account of the answer that
            // landed with an account of the tap that did not.
            return
        }
        GlancePermissionStore.write(claimed)

        guard let connection else {
            // `appName`, not a literal: a canary build is named "FC Canary" and
            // telling somebody running it to open "Far Cooler" sends them
            // looking for an app that is not on their phone.
            await settle(
                intent, .nothingSent,
                "Open \(appName) on your \(DeviceKind.current), then try again.")
            return
        }
        guard await ready(connection, within: Self.glanceConnectBudget) else {
            if case .needsApproval = connection.phase {
                await settle(
                    intent, .nothingSent,
                    "Approve this runner on your \(DeviceKind.current) first.")
            } else {
                await settle(
                    intent, .nothingSent,
                    "Your \(DeviceKind.current) can’t reach that runner right now.")
            }
            return
        }

        switch await pendingPermission(
            terminal: terminal, on: connection, within: Self.glanceReplayBudget)
        {
        case let .permission(pending):
            guard let pending else {
                // Nothing pending. NOT the same as "answered", and the sentence
                // must not say so: `PermissionView` spells out that an agent
                // can be blocked on something this vocabulary has no word for —
                // a trust gate, a plain question — and a restarted session
                // reads identically from here. What IS known is that the thing
                // these buttons offered to answer is not what the agent is
                // waiting on, so the buttons go and the sentence says only that.
                GlancePermissionStore.update { $0.clearingPermission(for: terminal) }
                await settle(intent, .nothingSent, "This agent isn’t waiting on that anymore.")
                return
            }
            guard pending.id == request else {
                // Answered somewhere else, and the agent has since stopped on
                // something new. The new permission is deliberately NOT written
                // here: this path is an answer, not an observation, and filing
                // fresh options under a claim made against the old ones is how
                // a card ends up offering buttons beside a sentence about a
                // different question. The next real observation records it.
                GlancePermissionStore.update { $0.clearingPermission(for: terminal) }
                await settle(
                    intent, .nothingSent, "This agent’s waiting on something else now.")
                return
            }
        case let .failed(reason):
            // The phone's own sentence, unedited — it already says whether this
            // is a runner it cannot reach or an agent it could not read in
            // time. Nothing was sent either way.
            await settle(intent, .nothingSent, reason)
            return
        case .sent, .transcript:
            // A receipt or a conversation in answer to a question about what is
            // pending is this build's own vocabulary contradicting itself. It
            // establishes nothing about what the agent is waiting on, so it must
            // not be treated as a match.
            await settle(
                intent, .nothingSent,
                "Your \(DeviceKind.current) couldn’t read that agent’s conversation.")
            return
        }

        do {
            _ = try await withTimeout(seconds: Self.glanceActionBudget) {
                try await connection.core.call(
                    "terminal.agent_answer",
                    ["terminal": terminal, "requestId": request, "optionId": intent.option])
            }
            // Cleared on success only, exactly as the watch's answer clears it,
            // and for the reason `Transcript.clearPendingPermission` gives: the
            // agent resumes without acknowledging what it was blocked on, so
            // nothing arrives to retire this and the next replay would re-offer
            // a permission already answered.
            replays[terminal]?.transcript.clearPendingPermission()
            GlancePermissionStore.update { $0.clearingPermission(for: terminal) }
            // The option's own name, quoted, because "Answered" alone does not
            // say which of several buttons landed — and on a lock screen the
            // tap and the confirmation can be minutes apart.
            await settle(intent, .sent, "Sent “\(intent.optionName)”.")
        } catch {
            let (outcome, message) = Self.glanceFailure(error)
            await settle(intent, outcome, message)
        }
    }

    /// Write the outcome where the card can read it, and ask the card to look.
    ///
    /// Awaited rather than launched in a `Task`, because the caller is an app
    /// intent and the process has no promise of living past its return: a
    /// detached redraw is a redraw that may never run, on exactly the pocketed
    /// phone this whole path exists for.
    private func settle(
        _ intent: AnswerPermissionIntent, _ outcome: GlanceAnswer.Outcome, _ message: String
    ) async {
        GlancePermissionStore.update {
            $0.settling(
                terminal: intent.terminal, request: intent.request,
                outcome: outcome, message: message, at: Date())
        }
        await Self.redraw(leading: intent.terminal)
    }

    /// The same three stages the watch gets, on a shorter clock.
    ///
    /// The clock is the difference and it is not adjustable: WatchConnectivity
    /// holds a reply handler open while the watch shows a spinner, and an app
    /// intent gets whatever background execution the system feels like granting
    /// a widget's button — which is not documented, is measured in seconds, and
    /// ends without warning. The watch's eight, ten and eight add to
    /// twenty-six, and an intent killed at twenty leaves a claim marked
    /// `inFlight` with nothing left running to settle it: a card reading
    /// "Sending your answer…" until `GlanceAnswer.freshFor` expires it.
    ///
    /// So each stage is cut, and cut in the direction that fails safely. A
    /// connect that gives up early reports `nothingSent`, which is true — the
    /// answer never left — and says it is safe to try again. Only the last
    /// stage can end in doubt, and it is the one nothing can shorten away.
    private static let glanceConnectBudget: TimeInterval = 5
    private static let glanceReplayBudget: TimeInterval = 7
    private static let glanceActionBudget: TimeInterval = 6

    /// What a failed answer means, in the only three words this surface has.
    ///
    /// The direction of the doubt is chosen, not incidental. Only the errors
    /// that PROVE the runner was never written to are reported as `nothingSent`;
    /// everything else, including a timeout and a link that vanished mid-call,
    /// is `unsure`. `withTimeout` says why in its own comment — it stops
    /// waiting, it does not stop the work, and "the call may well land a moment
    /// later, into nothing" — and `WatchLinkClient.didNotHearBack` says what
    /// that costs when it is guessed the other way: a reject that landed,
    /// reported as unsent, and a person who then taps the option that allows.
    ///
    /// `rejected` is the one error that is genuinely safe. It is the DAEMON's
    /// own refusal, returned through `DomainError`, which means the call
    /// arrived and was declined before any message reached the agent.
    private static func glanceFailure(_ error: Error) -> (GlanceAnswer.Outcome, String) {
        if error is Timeout {
            return (
                .unsure,
                "No answer came back. It may still have gone through — check before trying again."
            )
        }
        if let core = error as? ClientCore.CoreError {
            switch core {
            case .notStarted:
                return (
                    .nothingSent,
                    "Your \(DeviceKind.current) couldn’t start its connection. Nothing was sent."
                )
            case let .rejected(message):
                // The daemon answers `NotFound` for a terminal it no longer has,
                // which is worth its own sentence: nothing is wrong with the
                // link and trying again will not help.
                return (
                    .nothingSent,
                    message.lowercased().contains("not found")
                        ? "That agent isn’t running anymore."
                        : "That runner turned it down, so nothing was sent."
                )
            case .disconnected, .malformed:
                break
            }
        }
        return (
            .unsure,
            "Your \(DeviceKind.current) lost touch with the runner. It may still have "
                + "gone through — check before trying again."
        )
    }

    /// Ask the lock screen card to draw itself again.
    ///
    /// The outcome of an answer lives in the App Group, and a card only reads
    /// that file when it renders. WidgetKit is documented to refresh an
    /// interactive surface once its intent finishes; this re-publishes the
    /// card's CURRENT state as well, so the sentence appears even if that
    /// refresh does not cover a Live Activity. Not observed on a device — see
    /// this task's report.
    ///
    /// Every field is copied from what the card already holds, `staleDate`
    /// included. This is a request to redraw and must not become a second
    /// author of the card's contents: the relay writes those, and a phone that
    /// started editing them would be the `line` field
    /// `AgentActivityAttributes` documents — read by one side and written by
    /// two.
    ///
    /// Only the card that is leading with this agent, and never a card that has
    /// moved on to somebody else.
    private static func redraw(leading terminal: String) async {
        for activity in Activity<AgentActivityAttributes>.activities
        where activity.content.state.terminal == terminal {
            await activity.update(
                ActivityContent(
                    state: activity.content.state, staleDate: activity.content.staleDate))
        }
    }

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
    /// `budget` is passed rather than defaulted because the two callers do not
    /// share a clock: the watch is holding a `sendMessage` reply open, and an
    /// app intent has whatever background execution the system granted a
    /// widget's button. See `glanceConnectBudget`.
    private func ready(_ connection: Connection, within budget: TimeInterval) async -> Bool {
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
        let deadline = Date().addingTimeInterval(budget)
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
            return "Your \(DeviceKind.current) took too long. It may still have gone through — "
                + "check before trying again."
        }
        let text = error.localizedDescription.lowercased()
        if text.contains("not found") { return "That agent isn’t running anymore." }
        return "Couldn’t \(what). Your \(DeviceKind.current) lost touch with the runner."
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
            //
            // "iPhone" IS a literal here, and it is the one place in this file
            // that keeps one. This method is `nonisolated` and `UIDevice` is
            // main-actor-isolated in the SDK, so `DeviceKind` cannot be read
            // from it — and `replyHandler` has a watch holding a continuation
            // open, so hopping actors to read a noun is not a trade worth
            // making. It is also the one sentence that is provably about an
            // iPhone: a `WCSessionDelegate` callback only arrives on a device
            // where `WCSession.isSupported()` was true, which iPad is not.
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
