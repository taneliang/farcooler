import Foundation
import Testing

@testable import AgentKit

/// The fingerprint question, the way out of it, and the way back.
///
/// Every link in that circle is a string match or a table lookup, and not one
/// of them fails loudly on its own: a reworded sentence stops classifying, a
/// dropped case stops being drawn, and a move that forgets a key nobody pinned
/// is a button that runs and does nothing. This suite is what makes each of
/// them go red.
struct HostKeyQuestionTests {
    // MARK: - Every answer survives

    /// **The ruling.** The port replaced a full-screen approval phase with a
    /// row and shipped two of the three answers, so somebody unsure about a
    /// fingerprint could agree to it or force-quit.
    @Test func theQuestionOffersAllThreeAnswers() {
        #expect(
            HostKeyQuestion.Answer.allCases == [.trustThisRunner, .notNow, .editTheRunner])
    }

    /// The one that was lost, named on its own so a diff that drops it names
    /// itself.
    @Test func backingOutIsOffered() {
        #expect(HostKeyQuestion.Answer.allCases.contains(.notNow))
    }

    /// Title case, as every button in this app is, and the exact words the
    /// screen this replaced used.
    @Test func theAnswersAreTitleCased() {
        #expect(
            HostKeyQuestion.Answer.allCases.map(\.label)
                == ["Trust This Runner", "Not Now", "Edit…"])
    }

    /// Exactly one answer carries the weight. Two primaries is the failure
    /// `RunnerTrouble` already names about its own alternatives.
    @Test func exactlyOneAnswerIsTheAnswer() {
        #expect(HostKeyQuestion.Answer.allCases.filter(\.isTheAnswer) == [.trustThisRunner])
    }

    // MARK: - The circle

    /// Backing out has to land somewhere the app can name. The sentence is
    /// written in one file and matched in the same one, and this is what stops
    /// a reword turning a decision somebody made into "Can't Connect".
    @Test func decliningClassifiesAsKeyNotTrusted() {
        let said = RunnerTrouble.Said.declined(runner: "box.local")
        #expect(RunnerTrouble(message: said) == .keyNotTrusted)
        #expect(RunnerTrouble(message: said) == HostKeyQuestion.declining)
    }

    /// Not a fault, and it must not be dressed as one — no red, and no "Try
    /// Again" for a question that is simply still open.
    @Test func decliningIsNotHeadlinedAsAFault() {
        #expect(HostKeyQuestion.declining.isAlarming == false)
        #expect(HostKeyQuestion.declining.nextMove == .showTheKeyAgain)
        #expect(HostKeyQuestion.declining.worthRetryingAsAlternative == false)
    }

    /// **The way back has to actually go back.** Forgetting a key is only a
    /// reconnect when there was a key to forget, and after declining there
    /// never was one — the fingerprint is already nil, `FleetMembership.plan`
    /// files the runner under `kept`, and nothing re-dials. So this move
    /// carries the dial itself.
    @Test func showingTheKeyAgainDialsAndDoesNotOnlyForget() {
        #expect(RunnerTrouble.NextMove.showTheKeyAgain.acts == [.forgetThePinnedKey, .dialAgain])
    }

    /// The other half of the same rule, and the reason it is not "always dial":
    /// a host key that CHANGED really is pinned, so clearing it is a change to
    /// the runner and the reconcile rebuilds on its own. Dialing here too would
    /// be a second connect racing the rebuild for one runner id.
    @Test func reviewingAChangedKeyOnlyForgetsBecauseTheRebuildDials() {
        #expect(RunnerTrouble.NextMove.reviewTheNewKey.acts == [.forgetThePinnedKey])
    }

    /// The circle closed: decline, land, and the move offered there gets you
    /// back to a fingerprint on screen.
    @Test func theWayBackFromDecliningReachesTheQuestionAgain() {
        let kind = RunnerTrouble(message: RunnerTrouble.Said.declined(runner: "box.local"))
        let acts = kind.nextMove.acts
        #expect(acts.contains(.forgetThePinnedKey))
        #expect(acts.contains(.dialAgain))
    }

    // MARK: - Every move does something

    /// No move is empty. A labeled button wired to nothing is the defect this
    /// whole table exists to prevent, and it is invisible in a build.
    @Test func everyMoveHasSomethingToDo() {
        let moves: [RunnerTrouble.NextMove] = [
            .authorizeThisDevice, .reviewTheNewKey, .showTheKeyAgain, .addThisDeviceAgain,
            .tryAgain,
        ]
        for move in moves {
            #expect(!move.acts.isEmpty, "\(move.label) is a button that does nothing")
        }
    }

    /// The two key offers go to the same screen, because this device asking to
    /// be added again shows an offer carrying a node key a runner can admit.
    @Test func bothKeyOffersShowThisDevicesKey() {
        #expect(RunnerTrouble.NextMove.authorizeThisDevice.acts == [.offerThisDevicesKey])
        #expect(RunnerTrouble.NextMove.addThisDeviceAgain.acts == [.offerThisDevicesKey])
    }

    /// A move that offers this device's key never also dials: the dial would
    /// use the key that has not been installed yet, so it could only fail.
    @Test func offeringAKeyNeverAlsoDials() {
        for move in [RunnerTrouble.NextMove.authorizeThisDevice, .addThisDeviceAgain] {
            #expect(!move.acts.contains(.dialAgain))
        }
    }

    // MARK: - The sentences the app writes

    /// Each of the four sentences this app composes itself, read back through
    /// the classifier that has to recognize it. A reword breaks exactly one of
    /// these, by name.
    @Test func everySentenceTheAppWritesClassifiesBack() {
        #expect(
            RunnerTrouble(message: RunnerTrouble.Said.stoppedWaiting(for: "10.0.0.4")) == .stopped)
        #expect(RunnerTrouble(message: RunnerTrouble.Said.noIdentity) == .noIdentity)
        #expect(RunnerTrouble(message: RunnerTrouble.Said.noNodeKey) == .noNodeKey)
        #expect(
            RunnerTrouble(message: RunnerTrouble.Said.declined(runner: "box")) == .keyNotTrusted)
    }

    /// The four are told APART, not merely recognized. A phrase generic enough
    /// to match two of them would pass the test above and still send somebody
    /// to the wrong button.
    @Test func theAppsFourSentencesAreDistinct() {
        let said = [
            RunnerTrouble.Said.declined(runner: "box"),
            RunnerTrouble.Said.stoppedWaiting(for: "box"),
            RunnerTrouble.Said.noIdentity,
            RunnerTrouble.Said.noNodeKey,
        ]
        let kinds = said.map { RunnerTrouble(message: $0) }
        #expect(kinds == [.keyNotTrusted, .stopped, .noIdentity, .noNodeKey])
        #expect(!kinds.contains(.other))
    }
}
