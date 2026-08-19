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
/// Every context received is written through `SnapshotStore` before it is
/// published, so the complication reads the same bytes this screen does. Two
/// copies of the fleet on one watch is two answers to "what is that agent
/// doing", and the whole point of a projection computed once on the host is
/// that there is only ever one.
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
        state = .resolve(snapshot: snapshot, reachable: session.isReachable)
    }

    /// A received snapshot, stored where every watch surface reads it.
    ///
    /// The write happens before `state` is published, so the complication and
    /// the app can never be one update apart — and `reloadAllTimelines` is
    /// here because a complication does not poll. Without it the widget keeps
    /// drawing the previous snapshot until the system next decides to refresh
    /// it, which can be an hour: a wrist that says "working" long after the
    /// agent blocked. It is a no-op until Task 6 adds a widget to reload.
    ///
    /// Takes a decoded `FleetSnapshot` rather than the raw context, and that is
    /// not only tidiness: `[String: Any]` is not `Sendable`, so handing the
    /// dictionary across to the main actor is a data race the Swift 6 language
    /// mode rejects outright. `FleetSnapshot` is a `Sendable` value type, so
    /// decoding on the delegate's own queue is both sound and less work for the
    /// actor that is drawing.
    private func receive(_ snapshot: FleetSnapshot) {
        SnapshotStore.write(snapshot)
        WidgetCenter.shared.reloadAllTimelines()
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
