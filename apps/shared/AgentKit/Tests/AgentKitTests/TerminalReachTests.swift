import Foundation
import Testing

@testable import AgentKit

/// What a wrist is told when the phone cannot find the pane it named.
///
/// The whole suite is about one window — the seconds between a scene existing
/// and the first fleet arriving — which the port made reachable and left with
/// no sentence of its own. None of it could go red before: `WatchLinkHost` is in
/// the iOS target, which CI compiles and never runs.
struct TerminalReachTests {
    // MARK: - Which cause

    /// The ordinary case: the pane was found and there is nothing to say.
    @Test func findingThePaneIsNotAMiss() {
        #expect(TerminalReach.miss(hasScene: true, everyRunnerHasAnswered: true, found: true) == nil)
    }

    /// Found beats everything. A pane that resolved while the rest of the fleet
    /// was still connecting is a pane this phone can act on.
    @Test func findingThePaneBeforeTheFleetHasSettledIsStillNotAMiss() {
        #expect(
            TerminalReach.miss(hasScene: false, everyRunnerHasAnswered: false, found: true) == nil)
    }

    /// No scene at all. There is no app-wide store to fall back to, so this is
    /// ruled out first and does not depend on anything else.
    @Test func noSceneIsTheFirstCauseRuledOut() {
        #expect(
            TerminalReach.miss(hasScene: false, everyRunnerHasAnswered: false, found: false)
                == .noScene)
        #expect(
            TerminalReach.miss(hasScene: false, everyRunnerHasAnswered: true, found: false)
                == .noScene)
    }

    /// **The cold launch, and the bug.** A scene exists — `adopt` happens at
    /// scene creation now — and the fleet has not come back. The port answered
    /// this "isn't connected to the runner that pane is on", which is a
    /// confident wrong answer about a phone that simply had not looked yet.
    @Test func aSceneWithNoFleetYetIsStillStarting() {
        #expect(
            TerminalReach.miss(hasScene: true, everyRunnerHasAnswered: false, found: false)
                == .stillStarting)
    }

    /// Every runner has answered and none has this pane. Now the sentence about
    /// not being connected to its runner is true.
    @Test func aSettledFleetWithoutThePaneIsNotOnAnyRunner() {
        #expect(
            TerminalReach.miss(hasScene: true, everyRunnerHasAnswered: true, found: false)
                == .notOnAnyRunner)
    }

    /// **Every runner, not any.** A fleet of three where two have answered
    /// cannot say a pane does not exist — it may be on the third. This is the
    /// predicate `FleetView.dropUnknownTerminal` already reaches for, and using
    /// the weaker one here is the same defect one step later.
    @Test func aPartlyAnsweredFleetIsStillStartingAndNotAnAnswer() {
        let partly = TerminalReach.miss(
            hasScene: true, everyRunnerHasAnswered: false, found: false)
        #expect(partly == .stillStarting)
        #expect(partly != .notOnAnyRunner)
    }

    // MARK: - Whether waiting helps

    /// Exactly one of the three is worth trying again in a second. Telling
    /// somebody to retry something that cannot change is as bad as telling them
    /// nothing.
    @Test func exactlyOneCauseIsWorthRetryingSoon() {
        #expect(TerminalReach.Miss.allCases.filter(\.isWorthRetryingSoon) == [.stillStarting])
    }

    // MARK: - The sentences

    /// Each cause says something different. The bug was two causes sharing one
    /// sentence and a third sharing it by accident.
    @Test func everyCauseHasItsOwnSentence() {
        let said = TerminalReach.Miss.allCases.map {
            TerminalReach.sentence($0, appName: "Far Cooler", deviceKind: "iPhone")
        }
        #expect(Set(said).count == TerminalReach.Miss.allCases.count)
    }

    /// The app is named by whatever it is actually called. A canary build is
    /// "FC Canary", and sending somebody to open "Far Cooler" sends them
    /// looking for an app that is not on their phone.
    @Test func everySentenceNamesTheAppItWasGiven() {
        for miss in TerminalReach.Miss.allCases {
            let said = TerminalReach.sentence(miss, appName: "FC Canary", deviceKind: "iPhone")
            #expect(said.contains("FC Canary"))
            #expect(!said.contains("Far Cooler"))
        }
    }

    /// The two that name a device name the one the person is holding.
    @Test func theSentencesAboutThisPhoneNameTheDeviceKind() {
        for miss in [TerminalReach.Miss.noScene, .stillStarting] {
            let said = TerminalReach.sentence(miss, appName: "Far Cooler", deviceKind: "iPad")
            #expect(said.contains("iPad"))
        }
    }

    /// The cold-launch sentence says what is happening and what to do, and
    /// promises nothing about whether the pane exists — the phone does not know
    /// yet.
    @Test func theStillStartingSentenceOffersWaiting() {
        let said = TerminalReach.sentence(
            .stillStarting, appName: "Far Cooler", deviceKind: "iPhone")
        #expect(said.contains("still starting up"))
        #expect(said.contains("Try again in a moment."))
        #expect(!said.contains("isn’t connected"))
    }

    /// The settled sentence is the one the port already had, word for word,
    /// because it was right about the case it is now reserved for.
    @Test func theSettledSentenceIsUnchanged() {
        #expect(
            TerminalReach.sentence(.notOnAnyRunner, appName: "Far Cooler", deviceKind: "iPhone")
                == "Far Cooler isn’t connected to the runner that pane is on.")
    }

    /// Read on a wrist, in a sentence — typographic apostrophes, and no raw
    /// error text from anywhere.
    @Test func everySentenceIsProse() {
        for miss in TerminalReach.Miss.allCases {
            let said = TerminalReach.sentence(miss, appName: "Far Cooler", deviceKind: "iPhone")
            #expect(!said.contains("'"), "\(miss) uses a typewriter apostrophe")
            #expect(said.hasSuffix("."), "\(miss) is not a sentence")
        }
    }
}
