import Foundation
import WatchConnectivity
import WidgetKit

/// Everything the watch's screens are allowed to know about where data comes
/// from.
///
/// A protocol with exactly one shipping implementation, which is not
/// over-abstraction: the screens must not learn that the fleet arrives over
/// WatchConnectivity, because the moment they do, the standalone core this app
/// may one day carry becomes a rewrite of every view rather than a second
/// conformance. It is also what lets a preview hand a screen a fixed fleet
/// without a paired phone in the room.
@MainActor
protocol FleetClient: ObservableObject {
    var state: WatchState { get }
    /// Ask the phone to do one thing. Never throws: a watch has nowhere to put
    /// an error but the screen, so the failure IS the answer — see
    /// `WatchReply.failed`, which carries a sentence rather than a code.
    func send(_ request: WatchRequest) async -> WatchReply
}

extension FleetClient {
    /// Ask the phone for something, refusing before it leaves the watch when the
    /// phone cannot be reached.
    ///
    /// **Screens call this and never `send` directly.** The reachability rule is
    /// already stated three times — `WatchState.canAct` decides it, the buttons
    /// are disabled by it, and `WatchLinkClient.send` enforces it at the door —
    /// and this fourth statement is the one that closes the gap between the
    /// other three. Reachability can drop between the render that enabled a
    /// button and the tap that follows it, and a `FleetClient` that is not
    /// `WatchLinkClient` — a preview today, a standalone core one day — has no
    /// door of its own to enforce anything at.
    ///
    /// A screen that can send when it must not is the exact failure the whole
    /// reachability rule exists to prevent, and it fails silently: the person
    /// taps, sees a success, and walks away from an agent that is still waiting.
    ///
    /// The sentence is `WatchLinkClient.unreachable`, the same one the door
    /// produces. One condition that produced two different messages would be two
    /// conditions as far as anybody reading a watch is concerned.
    func attempt(_ request: WatchRequest) async -> WatchReply {
        guard state.canAct else { return .failed(WatchLinkClient.unreachable) }
        return await send(request)
    }
}

/// The watch's half of the link: receives the fleet, and asks the phone to act.
///
/// Two directions with two different mechanisms, deliberately:
///
///   - State arrives by `updateApplicationContext`. Latest-wins, delivered when
///     the system feels like it, and a stale copy never queues behind a fresh
///     one — which is a snapshot's semantics exactly. A message queue would
///     deliver five old fleets in order before the current one.
///   - Actions go by `sendMessage`, which wakes the phone app in the background
///     if it is not running, and which carries a reply. Nothing here is
///     fire-and-forget: a prompt that did not arrive has to say so.
///
/// Every context received is written through `SnapshotStore`, so the
/// complication reads the same bytes this screen does. Two copies of the fleet
/// on one watch is two answers to "what is that agent doing", and the whole
/// point of a projection computed once on the host is that there is only ever
/// one. The write happens in `SnapshotSink`, off this actor — see `receive`
/// for what that changed and what it deliberately did not.
@MainActor
final class WatchLinkClient: NSObject, ObservableObject, FleetClient {
    /// One per app. `WCSession.default` is a singleton with ONE delegate, so a
    /// second instance would silently steal delivery from the first — the
    /// screens would go quiet with no error anywhere.
    static let shared = WatchLinkClient()

    @Published private(set) var state: WatchState = .nothing

    /// The last fleet we were told about, kept apart from `state` because
    /// reachability changes without the fleet changing and must not blank it.
    private var snapshot: FleetSnapshot?

    private let session = WCSession.default
    private var started = false

    /// Where an arriving snapshot is written to disk and where the
    /// complication is reloaded from — neither on this actor. See
    /// `SnapshotSink`.
    private let sink = SnapshotSink()

    /// How many snapshots have been handed to `sink`. It is what keeps them in
    /// order once they leave this actor; see `SnapshotSink.store`.
    private var received: UInt64 = 0

    private override init() { super.init() }

    /// The one sentence for "the phone is not there right now".
    ///
    /// Said in one place because it is produced in two: the door below, and
    /// `FleetClient.attempt` before a request ever reaches the door. Two
    /// wordings for one condition would read as two different problems to
    /// somebody who saw both in a minute, and they would go looking for the
    /// difference.
    static let unreachable = "Can’t reach your iPhone. Try again when it’s nearby."

    /// The one sentence for "the phone answered, but not with anything this
    /// build's vocabulary has a case for" — a version skew between watch and
    /// phone, or between what a screen asked and what the reply actually was.
    ///
    /// Said once because it is reached from four places that are each a
    /// different mismatch — this file's own undecodable reply, a `.sent`
    /// where `PermissionView.ask` expected `.permission`, a `.permission`
    /// where `PermissionView.answer` and `ComposeView.send` expected `.sent`
    /// — and all four read as the same problem to somebody looking at a
    /// watch face. Two wordings of one problem is how a person notices the
    /// seam between the screens that share it, which is exactly what
    /// `unreachable`, above, already exists to prevent for its condition.
    static let unreadableReply = "Your iPhone answered with something this watch can’t read."

    /// The one sentence for "we never heard back", which is NOT the same as
    /// "it did not happen".
    ///
    /// `sendMessage`'s error handler fires when the REPLY fails to arrive, and
    /// a reply that never arrived says nothing about the message: the phone may
    /// have taken it, performed it, and answered a moment too late. The phone
    /// is explicit about the same thing one layer down —
    /// `WatchLinkHost.withTimeout` stops waiting without stopping the work, so
    /// "the call may well land a moment later, into nothing."
    ///
    /// So this sentence must not claim the work did not happen. It says what is
    /// actually known and what to do about it. The failure it exists to prevent
    /// is concrete: a **Deny** that landed, reported as unsent, and the person
    /// taps **Allow** — the opposite of what they chose, on the one screen
    /// where that cannot be taken back. `PermissionView` disables its buttons
    /// for the whole of a send to stop exactly that, then hands them back on a
    /// failure, which leaves this sentence as the last thing standing between a
    /// late reply and a reversed answer.
    private static let didNotHearBack =
        "Your iPhone didn’t answer. It may still have gone through — "
        + "check before trying again."

    /// The sentence for the errors that fail BEFORE the phone is handed
    /// anything.
    ///
    /// Worth telling apart from `didNotHearBack`, because this is the case
    /// where trying again costs nothing and changes nothing — and saying so is
    /// the difference between a person retrying and a person picking up their
    /// phone for no reason.
    private static let nothingSent =
        "Your iPhone didn’t take that. Nothing was sent, so it’s safe to try again."

    /// What a failed `sendMessage` means to the person holding the watch.
    ///
    /// The system's error is a `WCError` code, which is not language — but the
    /// codes are not interchangeable either, and this used to collapse all of
    /// them into "Nothing was sent." Two groups differ in the only way a watch
    /// face has to be honest about:
    ///
    ///   - `.messageReplyTimedOut` and `.messageReplyFailed` are about the
    ///     REPLY. The message may already have been delivered and performed.
    ///   - `.deliveryFailed` and the codes beside it fail at this end, before
    ///     the phone sees anything, so they can honestly say nothing was sent.
    ///
    /// Anything left over — `.genericError` most of all — is unknown, and
    /// unknown is reported as unknown rather than guessed in the direction that
    /// happens to read better.
    private nonisolated static func sendFailure(_ error: Error) -> String {
        guard let code = (error as? WCError)?.code else { return didNotHearBack }
        switch code {
        case .notReachable:
            // One condition, one wording — the reason `unreachable` is a
            // property at all. Somebody who saw two sentences for a phone that
            // walked out of range would go looking for the difference.
            return unreachable
        case .deliveryFailed, .payloadTooLarge, .payloadUnsupportedTypes,
            .invalidParameter, .sessionNotActivated, .sessionInactive,
            .sessionMissingDelegate, .companionAppNotInstalled:
            return nothingSent
        default:
            return didNotHearBack
        }
    }

    /// Activate, and put the last known fleet on screen before anything
    /// arrives.
    ///
    /// The disk read first is what stops a cold launch from showing "nothing
    /// known" for the second or two activation takes. That empty state means
    /// "this watch has never heard from the phone", and showing it to somebody
    /// whose watch has a perfectly good snapshot on it is a lie with a delay
    /// on it.
    func start() {
        guard WCSession.isSupported(), !started else { return }
        started = true
        adopt(SnapshotStore.read())
        session.delegate = self
        session.activate()
    }

    func send(_ request: WatchRequest) async -> WatchReply {
        guard session.activationState == .activated else {
            return .failed("The watch app is still starting. Try again in a moment.")
        }
        // The same rule `WatchState.canAct` states, enforced at the door rather
        // than trusted to the caller. A screen that got its enabled/disabled
        // logic wrong then produces a sentence someone can read instead of a
        // tap that quietly goes nowhere.
        guard session.isReachable else { return .failed(Self.unreachable) }
        return await withCheckedContinuation { continuation in
            // Exactly one of these two runs, per WatchConnectivity's contract.
            // If that ever stopped being true the second resume would trap, and
            // a trap is the right outcome: a continuation resumed twice has
            // already handed one screen two different answers.
            session.sendMessage(
                request.dictionary,
                replyHandler: { reply in
                    // An unreadable reply is a phone speaking a dialect this
                    // build does not know — a version skew — and not a failure
                    // of the thing that was asked. Saying so beats reporting
                    // success for something we cannot confirm happened.
                    continuation.resume(
                        returning: WatchReply(dictionary: reply)
                            ?? .failed(Self.unreadableReply))
                },
                errorHandler: { error in
                    continuation.resume(returning: .failed(Self.sendFailure(error)))
                })
        }
    }

    /// Take a snapshot as the current truth, then recompute what may be shown.
    private func adopt(_ snapshot: FleetSnapshot?) {
        if let snapshot { self.snapshot = snapshot }
        recompute()
    }

    /// The three states, from the two facts that decide them.
    ///
    /// The decision itself is `WatchState.resolve`, in AgentKit, because that
    /// is where a test can reach it. What is left here is reading
    /// `isReachable` — which no test can fake, and which is the one thing this
    /// file is uniquely able to know.
    private func recompute() {
        let next = WatchState.resolve(snapshot: snapshot, reachable: session.isReachable)
        // Assigned only when it differs, because two of the three callers
        // routinely call this with nothing having changed.
        // `sessionReachabilityDidChange` is the plain case: WatchConnectivity
        // fires it whenever it re-evaluates reachability, including when the
        // answer is the same one as before, and a `@Published` assignment is a
        // change as far as SwiftUI is concerned whether or not the VALUE
        // changed. Every screen observing this client then rebuilds its body —
        // `FleetListView`, the detail screen behind it, and the `TimelineView`
        // schedules both of them now derive from the snapshot — for a
        // reachability callback that said what the last one said.
        //
        // `WatchState` is `Equatable` and its equality is the snapshot's, so
        // this is a deep compare of the fleet. That is the same compare
        // `WatchLinkHost.send` already makes on the phone before every push,
        // and it is bounded by the fleet's size; a body evaluation is not.
        guard next != state else { return }
        state = next
    }

    /// A received snapshot, published to the screens and handed to the sink
    /// that stores it.
    ///
    /// **Neither the disk write nor the complication reload happens here any
    /// more.** Both used to run inline on this actor, on every context that
    /// landed, and `WatchLinkHost.send` lands one every three seconds for as
    /// long as any agent is working — its own comment says why, and says the
    /// cadence is right: `line` and `feed` are part of `Agent`'s equality and
    /// churn on nearly every poll, so the "unchanged agents are not resent"
    /// guard buys an idle fleet and nothing else. What that cadence landed on
    /// was a JSON encode, an `.atomic` file replace, and a cross-process
    /// `reloadAllTimelines`, three seconds apart, on the actor drawing the
    /// fleet list. The cadence stays; `SnapshotSink` is where the three now
    /// run.
    ///
    /// One ordering claim moved and one was dropped, and the difference
    /// matters:
    ///
    ///   - **Kept: the file is written before the complication is asked to
    ///     reload.** `SnapshotSink.store` does the two in that order on one
    ///     serialized executor. Reversed, the extension opens the file, finds
    ///     the PREVIOUS snapshot, and draws it — a reload that spends a
    ///     watchOS budget to render exactly what was already on the face.
    ///   - **Dropped: that the write precedes the publish.** This used to say
    ///     "the complication and the app can never be one update apart",
    ///     which was a true description of two adjacent lines rather than a
    ///     rule anything depended on. It cannot survive moving the write off
    ///     the drawing actor, and nothing needs it to: the complication is a
    ///     separate process that reads the file only when it is reloaded, so
    ///     the window this opens is between a screen that is on and a face
    ///     that is not being redrawn. `reloadAllTimelines` is throttled now in
    ///     any case, so the two surfaces are already allowed to be a moment
    ///     apart — see `SnapshotSink.Complication`.
    ///
    /// `seq` is assigned here, on the actor where the deliveries are already
    /// in order, because the sink is not that actor: two `Task`s awaiting the
    /// same actor are not promised to run in the order they were created, and
    /// a stale snapshot landing in the file after a fresh one would leave the
    /// face an update behind until the next context, up to thirty seconds
    /// later.
    ///
    /// Takes a decoded `FleetSnapshot` rather than the raw context, and that is
    /// not only tidiness: `[String: Any]` is not `Sendable`, so handing the
    /// dictionary across to the main actor is a data race the Swift 6 language
    /// mode rejects outright. `FleetSnapshot` is a `Sendable` value type, so
    /// decoding on the delegate's own queue is both sound and less work for the
    /// actor that is drawing.
    private func receive(_ snapshot: FleetSnapshot) {
        received += 1
        let seq = received
        Task { await sink.store(snapshot, seq: seq) }
        adopt(snapshot)
    }

    /// The context's one key, and the coding on both sides of it.
    ///
    /// JSON `Data` under a single key rather than a hand-coded property list.
    /// `Data` IS a property-list type, so `WCSession` carries it, and
    /// `FleetSnapshot` is already `Codable` — hand-coding eleven fields twice
    /// would be a second projection of the pane, which is the one thing this
    /// design forbids.
    ///
    /// **Both coders are deliberately left at their defaults**, here and in
    /// `WatchLinkHost.encode`. A configured pair has to be configured
    /// identically in two files, and the way that fails is not a build error:
    /// `.secondsSince1970` encoded and read back as the default strategy
    /// decodes without complaint and lands every date thirty-one years out, so
    /// a fresh fleet would render as a day-old one. Nothing to configure is
    /// nothing to keep in step. This format is the wire's alone — `SnapshotStore`
    /// pins its own strategy for the file, and the two never meet.
    nonisolated static let snapshotKey = "snapshot"

    /// `nonisolated` so the delegate's own queue can run it. Pure — a
    /// dictionary in, a value type out — so there is nothing for the main actor
    /// to protect here, and requiring it would put JSON parsing on the thread
    /// that is drawing.
    private nonisolated static func decode(_ context: [String: Any]) -> FleetSnapshot? {
        guard let data = context[snapshotKey] as? Data else { return nil }
        return try? JSONDecoder().decode(FleetSnapshot.self, from: data)
    }
}

/// The two things an arriving snapshot costs, done off the actor that is
/// drawing.
///
/// An `actor` rather than a detached task per snapshot, because both jobs carry
/// state that has to be consistent with the other: what is in the file, and
/// what the complication was last reloaded for. A task per snapshot would give
/// each of them a copy of that state.
///
/// It is deliberately not `@MainActor`. `SnapshotStore.write` JSON-encodes the
/// fleet and does a `.atomic` replace — a temporary file, a write, and a rename
/// — and `reloadAllTimelines` is a cross-process call into the widget daemon.
/// Neither is a computation the screen needs the answer to, and both used to
/// run on the actor drawing the fleet list at `WatchLinkHost`'s three-second
/// push cadence.
///
/// The atomic write is kept exactly as it was. A half-written snapshot is a
/// surface showing a state that was never true, which is the one thing none of
/// these surfaces may do — `SnapshotStore`'s own comment makes the same point,
/// and moving the call to another executor does not change what it has to be.
private actor SnapshotSink {
    /// The highest `seq` written so far, and the whole of the ordering
    /// guarantee. See `store`.
    private var stored: UInt64 = 0

    /// What the complication was last reloaded FOR, or nil before the first
    /// reload of this launch.
    private var reloaded: Complication?

    /// Write the fleet, then reload the complication if it would draw
    /// something different.
    ///
    /// Ordered by `seq` rather than by arrival. Jobs enqueued on an actor are
    /// not promised to run in the order they were enqueued, and this one is
    /// enqueued from `WatchLinkClient.receive` where the deliveries genuinely
    /// are ordered. An older snapshot overwriting a newer one leaves a file
    /// that is not broken but is a whole fleet out of date, and nothing would
    /// correct it until the next context arrives — up to thirty seconds
    /// later, and longer than that once the phone goes out of range.
    ///
    /// `capturedAt` is deliberately NOT the thing compared. It comes off the
    /// phone's clock, and a clock that stepped backwards would block every
    /// write until it caught up — a complication frozen for as long as the
    /// skew lasts. A counter this process assigns cannot do that.
    func store(_ snapshot: FleetSnapshot, seq: UInt64) {
        guard seq > stored else { return }
        stored = seq
        SnapshotStore.write(snapshot)

        // Before the reload, and that order is the one rule this file inherited
        // from the two lines it replaced: the extension opens the file when it
        // is reloaded, so a reload asked for first draws the previous snapshot
        // and spends a watchOS refresh budget rendering what was already there.
        let complication = Complication(snapshot)
        guard complication != reloaded else { return }
        reloaded = complication
        WidgetCenter.shared.reloadAllTimelines()
    }

    /// Everything `WatchFleetWidget` can actually put on a face, and nothing
    /// else off the snapshot.
    ///
    /// **This is the throttle, and it is a change test rather than a clock.**
    /// The two candidates were "reload when what the complication draws
    /// changes" and "reload at most every thirty seconds"; this is the first,
    /// and the reason is what the surface is for. A `blocked` agent is the one
    /// thing somebody wants off a wrist without asking, and a floor would hold
    /// that back by up to thirty seconds on the one transition the whole
    /// feature exists to deliver. A change test costs that nothing: the reload
    /// still goes out the instant the top agent changes, and the case it
    /// removes — an agent working away with `line` and `feed` churning under a
    /// headline that has not moved — is precisely the one with nothing to show
    /// for it.
    ///
    /// No periodic reload is needed underneath it. The complication's timeline
    /// carries one entry per moment in `stalenessMoments`, so it stops
    /// asserting `working` on its own with nothing arriving — that is what
    /// `WatchFleetProvider.getTimeline` builds and why its policy is `.never`.
    ///
    /// The fields are listed one at a time instead of comparing `Agent`
    /// values, and that IS the throttle rather than fussiness: `feed` and
    /// `rank` are part of `Agent`'s equality and `feed` churns on nearly every
    /// poll, so comparing agents would find a difference every three seconds
    /// and suppress nothing. `feed` reaches no watch face — no family here
    /// draws it — so a change to it is not a change to this surface.
    private struct Complication: Equatable {
        /// `ranked.first`'s fields — the one agent every family renders. The
        /// four text fields are all of `agentTitle`'s fallback ladder, kept as
        /// raw fields so this does not become a fourth copy of the ladder
        /// itself.
        let id: String?
        let glyph: String
        let headline: String
        let line: String
        let label: String
        let status: String

        /// The number `Circular` draws.
        let needingYou: Int

        /// `WatchFleetEntry.hasSnapshot`'s epoch test, which changes what the
        /// text families SAY: "Open <app>" before anything has ever been
        /// written, "No agents" after a real capture came back empty. Needed
        /// on its own because that is the one transition every other field
        /// here can sit through unchanged — a first snapshot with no agents in
        /// it leaves all of them empty and the count at zero.
        let hasSnapshot: Bool

        /// When this snapshot stops vouching for each agent, which is what the
        /// timeline's entries are. A snapshot whose expiries have moved has to
        /// reload even when the top agent reads identically, or the face keeps
        /// a timeline built for the previous one.
        ///
        /// Measured from `capturedAt` rather than from `Date()` so that one
        /// snapshot always yields one answer here — a wall clock would make
        /// this differ from itself between two calls and defeat the compare.
        ///
        /// **Rounded to `momentGrain`, and that rounding is now load-bearing.**
        /// These moments are `lastHeard(of:)` plus an hour, and `lastHeard` is
        /// `observedAt` — which the phone stamps at every poll for every agent,
        /// deliberately, because that is what makes a long-running agent stop
        /// reading as "last seen working". So an exact compare finds a
        /// difference in this field on every single snapshot that lands, and
        /// this throttle would suppress nothing at all: `reloadAllTimelines`
        /// every three seconds while anything is working, and every thirty on
        /// a fleet where nothing is happening at all.
        ///
        /// Rounding costs the face at most `momentGrain` of accuracy on when it
        /// stops asserting `working`, against a `staleAfter` of an hour. That
        /// is a fifth of a percent, on a judgement whose own threshold is a
        /// round number somebody picked. It buys back the property this whole
        /// struct exists for.
        ///
        /// This was previously noted as churning only for a daemon too old to
        /// send `activitySince`. That is no longer the interesting case — it is
        /// now every fleet — and the fix is here rather than on the host,
        /// because what changed is what this surface has to notice rather than
        /// what the host says.
        let moments: [Date]

        /// How finely the reload test reads an expiry. Five minutes.
        private static let momentGrain: TimeInterval = 5 * 60

        init(_ snapshot: FleetSnapshot) {
            // The host's order, not ours — `rank` with the id breaking ties,
            // which is the same `ranked.first` the complication itself asks
            // for. Sorting here is not a second definition of urgency; it is
            // the same call, made once per snapshot on this executor instead
            // of on the one that draws.
            let top = snapshot.ranked.first
            id = top?.id
            glyph = top?.glyph ?? ""
            headline = top?.headline ?? ""
            line = top?.line ?? ""
            label = top?.label ?? ""
            status = top?.status ?? ""
            needingYou = snapshot.needingYou
            hasSnapshot = snapshot.capturedAt.timeIntervalSince1970 > 0
            // Down rather than to nearest, so a rounded moment is never later
            // than the real one: the face may stop asserting `working` a little
            // early, and must not go on asserting it a little late.
            moments = snapshot.stalenessMoments(after: snapshot.capturedAt).map {
                let grain = Self.momentGrain
                return Date(
                    timeIntervalSince1970: ($0.timeIntervalSince1970 / grain).rounded(.down)
                        * grain)
            }
        }
    }
}

/// The delegate half, `nonisolated` because WatchConnectivity calls back on its
/// own queue.
///
/// Every one of these hops to the main actor before touching a stored property.
/// Publishing from the delegate's thread is the classic SwiftUI crash — "must
/// be called on the main thread" — and it does not reproduce every time, which
/// is what makes it a shipping bug rather than a caught one.
///
/// What crosses that hop is a decoded `FleetSnapshot`, never the `[String: Any]`
/// the system handed over. The dictionary is not `Sendable` and sending one is
/// a data race the Swift 6 language mode rejects; decoding here also keeps the
/// JSON off the actor that is drawing.
extension WatchLinkClient: WCSessionDelegate {
    nonisolated func session(
        _ session: WCSession,
        activationDidCompleteWith activationState: WCSessionActivationState,
        error: Error?
    ) {
        // The context that arrived while this app was not running.
        //
        // `didReceiveApplicationContext` only fires for something delivered
        // while we are here to hear it. `receivedApplicationContext` is the
        // latest one the system is holding, and reading it on activation is
        // the difference between a watch that shows the fleet the moment it
        // opens and one that shows yesterday's until the phone next polls.
        let held = Self.decode(session.receivedApplicationContext)
        Task { @MainActor in
            if let held { self.receive(held) }
            self.recompute()
        }
    }

    nonisolated func session(
        _ session: WCSession, didReceiveApplicationContext applicationContext: [String: Any]
    ) {
        guard let snapshot = Self.decode(applicationContext) else { return }
        Task { @MainActor in self.receive(snapshot) }
    }

    nonisolated func sessionReachabilityDidChange(_ session: WCSession) {
        // The fleet has not changed; what it is worth has. `recompute` moves
        // between `.live` and `.cached` without touching the snapshot, which is
        // what keeps the rows on screen while the buttons under them go dead.
        Task { @MainActor in self.recompute() }
    }
}
