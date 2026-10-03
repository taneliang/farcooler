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
    /// A runner named the way `Runner.named` names one, which is what the two
    /// sentences below take. A tunneled runner has no address at all — see
    /// `RunnerTrouble.Said` — so a `String` was the wrong parameter and the
    /// bug it let through was invisible here.
    private func named(_ name: String, port: Int? = 22) -> RunnerTrouble.Words {
        RunnerTrouble.Words(name: name, reachDetail: "e@\(name)", port: port)
    }

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

    /// Backing out has to land somewhere the app can name, and it is named
    /// by kind, never read back out of the sentence (ov-127): no word the core
    /// can send lands there, so a decision somebody made cannot be mistaken
    /// for a failure the runner reported, or the other way round.
    @Test func decliningIsAKindNoCoreWordProduces() {
        #expect(HostKeyQuestion.declining == .keyNotTrusted)
        #expect(!RunnerTroubleTests.everyCoreWord.isEmpty, "the shared fixture did not load")
        for word in RunnerTroubleTests.everyCoreWord {
            #expect(RunnerTrouble(trouble: word) != HostKeyQuestion.declining, "\(word)")
        }
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
        #expect(
            RunnerTrouble.NextMove.showTheKeyAgain.acts == [.forgetThePinnedKey, .dialAgain])
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
        let acts = HostKeyQuestion.declining.nextMove.acts
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

    /// The four kinds this app raises itself are never a reading of the core's
    /// words. They are set beside their sentence in `Connection`; a core word
    /// that mapped onto one would let a runner's failure put "Not Now"'s
    /// landing, or "Stopped waiting", on a screen nobody chose.
    @Test func noCoreWordIsAKindTheAppRaises() {
        let raised: [RunnerTrouble] = [.keyNotTrusted, .stopped, .noIdentity, .noNodeKey]
        #expect(!RunnerTroubleTests.everyCoreWord.isEmpty, "the shared fixture did not load")
        for word in RunnerTroubleTests.everyCoreWord {
            #expect(!raised.contains(RunnerTrouble(trouble: word)), "\(word)")
        }
    }

    /// **The two sentences that name a runner name the runner.**
    ///
    /// A tunneled runner is named by the label somebody ticked, not by an
    /// address it does not have, and both of these were built out of
    /// `Runner.address` — empty for a tunnel — so they read "Stopped waiting
    /// for ." and "The key  presented has not been trusted on this device."
    ///
    /// The parameter is a `Words` now, which is what stops the address being
    /// passed at all. This is the other half: that what lands in the sentence
    /// is `name` and not one of the two fields beside it.
    @Test func theSentencesNameTheRunnerTheWayASentenceDoes() {
        let tunneled = RunnerTrouble.Words(
            name: "the spare room", reachDetail: "e, through the tunnel", port: nil)
        let stopped = RunnerTrouble.Said.stoppedWaiting(for: tunneled)
        #expect(stopped == "Stopped waiting for the spare room. It may be asleep or off the network.")
        let declined = RunnerTrouble.Said.declined(runner: tunneled)
        #expect(declined.hasPrefix("The key the spare room presented has not been trusted"))
        // The hole the address left, named so a regression is recognizable
        // rather than merely unequal.
        for sentence in [stopped, declined] {
            #expect(!sentence.contains("for ."), "the runner's name is missing: \(sentence)")
            #expect(!sentence.contains("key  presented"), "the runner's name is missing: \(sentence)")
        }
    }
}
