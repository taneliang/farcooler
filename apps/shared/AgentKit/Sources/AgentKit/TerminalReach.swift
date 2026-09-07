import Foundation

// Why a terminal id did not turn into a runner, and what to say about it.
//
// A request from a wrist or a lock-screen card names a TERMINAL and nothing
// else — the wire has never carried a runner — so the phone answers it by
// searching the merged fleet. When that search comes back empty, this is what
// the person is told.
//
// Here for `RunnerTrouble.swift`'s reason exactly: the sentence has to be
// somewhere a test can read it back, and `WatchLinkHost` lives in the iOS
// target, which CI compiles and never executes. Copy kept inside that target is
// copy nothing checks.
//
// **The port left this window half-converted, and the cost is a confident wrong
// answer on every cold launch.** The search reads `FleetStore.entries`, which is
// empty until the first poll returns — while the `fleet != nil` guard above it
// has already passed, because the store is adopted at SCENE CREATION now rather
// than when a connection succeeds. So a watch tap that wakes the phone, or a
// lock-screen answer that launches it, arrives in a window where the honest
// answer is "not yet" and the code had only two answers, neither of them that
// one. The comment there named two causes — the pane is on a runner this phone
// is not talking to, or it has stopped existing — and it was right that neither
// is something a person on a wrist can act on differently. It was missing the
// third, which is the only one they CAN act on, by waiting a second.
enum TerminalReach {
    /// Why the search came back empty.
    ///
    /// Three, and the order is the order they are ruled out in: no scene at
    /// all, then a fleet that has not answered yet, then a pane that is
    /// genuinely not on any runner this phone is talking to.
    enum Miss: Sendable, Equatable, Hashable, CaseIterable {
        /// No `FleetStore` — there is no app-wide one to fall back to.
        case noScene
        /// The app is up and the fleet has not come back yet. **The cause the
        /// port could not express**, and on a cold launch the most common one.
        case stillStarting
        /// Every runner has answered and none of them has this pane. It is on a
        /// runner this phone is not talking to, or it has stopped existing —
        /// two causes with one sentence, deliberately, because a person on a
        /// wrist cannot act on them differently and a sentence that guessed
        /// between them would be wrong half the time.
        case notOnAnyRunner

        /// Whether waiting a moment could change the answer.
        ///
        /// The distinction the whole file is for. One of these is worth trying
        /// again in a second and the other two are not, and telling somebody to
        /// retry something that cannot change is as bad as telling them nothing.
        var isWorthRetryingSoon: Bool { self == .stillStarting }
    }

    /// What went wrong, or nil if nothing did.
    ///
    /// `everyRunnerHasAnswered` and not "any runner has answered": a fleet of
    /// three where two have come back still cannot say a pane does not exist,
    /// because it might be on the third. `FleetView.dropUnknownTerminal` reaches
    /// for the same predicate for the same reason, and it is the one that was
    /// missing here.
    ///
    /// A phone with no runners configured answers `notOnAnyRunner`, which is
    /// literally true — there is no runner it is talking to — and is the honest
    /// thing to say rather than "still starting", which would promise that
    /// waiting helps when nothing is coming.
    static func miss(hasScene: Bool, everyRunnerHasAnswered: Bool, found: Bool) -> Miss? {
        guard !found else { return nil }
        guard hasScene else { return .noScene }
        return everyRunnerHasAnswered ? .notOnAnyRunner : .stillStarting
    }

    /// The sentence, read on a wrist or under a lock screen.
    ///
    /// `appName` rather than a literal: a canary build is named "FC Canary", and
    /// telling somebody running it to open "Far Cooler" sends them looking for
    /// an app that is not on their phone — a mistake already fixed once in the
    /// Live Activity and once in the widget. `deviceKind` for the same reason on
    /// the other noun.
    ///
    /// Two strings rather than the values themselves, because `DeviceKind` and
    /// the bundle are declared in the iOS target and this package cannot see
    /// them. The same shape `RunnerTrouble.Words` takes.
    static func sentence(_ miss: Miss, appName: String, deviceKind: String) -> String {
        switch miss {
        case .noScene:
            return "Open \(appName) on your \(deviceKind), then try again."
        case .stillStarting:
            // Says what is happening and what to do, and promises nothing about
            // whether the pane exists — because at this point the phone does
            // not know.
            return "\(appName) is still starting up on your \(deviceKind). "
                + "Try again in a moment."
        case .notOnAnyRunner:
            return "\(appName) isn’t connected to the runner that pane is on."
        }
    }
}
