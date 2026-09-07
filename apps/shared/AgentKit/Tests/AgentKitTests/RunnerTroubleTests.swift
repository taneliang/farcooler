import Foundation
import Testing

@testable import AgentKit

/// The claim step 3 of the multi-runner port rests on: moving the per-failure
/// logic out of a full-screen phase and into a row loses none of the next moves
/// it offered. Losing one would cost a user the only action that fixes their
/// runner, and nothing else in the tree would notice.
struct RunnerTroubleTests {
    private let words = RunnerTrouble.Words(
        name: "mini.local", reachDetail: "e@mini.local", port: 2222)
    private let tunneled = RunnerTrouble.Words(
        name: "the spare room", reachDetail: "e, through the tunnel", port: nil)

    // MARK: - Reading the core's message

    /// The substrings are the ones `crates/client/src/ssh.rs` and `session.rs`
    /// produce, each a distinctive phrase from the MIDDLE of its message rather
    /// than a prefix — so wrapping the error in more context does not stop it
    /// matching. Nothing tested this classifier before it moved here.
    @Test(arguments: [
        ("The host rejected this key.", RunnerTrouble.keyRejected),
        (
            "The key mini.local presented is not the one Far Cooler has recorded.",
            RunnerTrouble.hostKeyChanged
        ),
        ("Far Cooler cannot reach mini.local (os error 61)", RunnerTrouble.unreachable),
        ("The runner did not answer.", RunnerTrouble.daemonMissing),
        ("This device has no SSH key and one could not be generated.", RunnerTrouble.noIdentity),
        ("This device has no tunnel key.", RunnerTrouble.noNodeKey),
        (
            "The key mini.local presented has not been trusted on this device.",
            RunnerTrouble.keyNotTrusted
        ),
        ("Stopped waiting for mini.local.", RunnerTrouble.stopped),
    ])
    func aMessageIsClassifiedByThePhraseInsideIt(message: String, kind: RunnerTrouble) {
        #expect(RunnerTrouble(message: message) == kind)
    }

    /// The phrase is looked for anywhere in the message, not at the front.
    @Test func aWrappedMessageStillMatches() {
        let wrapped = "While connecting: the host rejected this key. (attempt 3)"
        #expect(RunnerTrouble(message: wrapped) == .keyRejected)
    }

    /// Anything with no phrase this app knows is `other`, which is the only
    /// kind that puts the core's own words on screen.
    @Test func anUnrecognizedMessageIsUndiagnosed() {
        #expect(RunnerTrouble(message: "kex_exchange_identification: banner line") == .other)
        #expect(RunnerTrouble(message: "").showsTheRunnersOwnWords)
    }

    /// **The bug this case exists for.** `tunnel_error` renders every tunnel
    /// failure but a refused port as `cannot open the tunnel: <word>`, which
    /// matches none of the phrases above — so a revoked device fell through to
    /// `other`, and `other` is the one kind that puts the core's own words on
    /// screen. The person read `cannot open the tunnel: no_answer`: a raw
    /// machine word, in the situation where a clear sentence matters most.
    @Test(arguments: [
        ("cannot open the tunnel: no_answer", RunnerTrouble.TunnelWord.noAnswer),
        ("cannot open the tunnel: derp", RunnerTrouble.TunnelWord.rendezvous),
        ("cannot open the tunnel: no_tailcat", RunnerTrouble.TunnelWord.notInThisBuild),
        ("cannot open the tunnel: io", RunnerTrouble.TunnelWord.unspecified),
    ])
    func aTunnelFailureIsClassifiedByItsStableWord(
        message: String, word: RunnerTrouble.TunnelWord
    ) {
        #expect(RunnerTrouble(message: message) == .tunnelFailed(word))
    }

    /// Every word `farcooler_tailcat::TunnelError::code` can send is one this
    /// app has a case for. A word added in Rust with no case here would land on
    /// `unspecified`, which is a sentence — never on `other`, which is the raw
    /// text.
    @Test func aWordThisBuildHasNeverSeenIsStillASentence() {
        #expect(
            RunnerTrouble(message: "cannot open the tunnel: quic")
                == .tunnelFailed(.unspecified))
        // Not even an empty one gets through as itself.
        #expect(RunnerTrouble(message: "cannot open the tunnel: ") == .tunnelFailed(.unspecified))
    }

    /// The word is read out of the middle of a wrapped message too, and it ends
    /// at the first space rather than swallowing whatever follows it.
    @Test func theWordIsReadOutOfAWrappedMessage() {
        #expect(
            RunnerTrouble(message: "While connecting: cannot open the tunnel: derp (attempt 3)")
                == .tunnelFailed(.rendezvous))
    }

    /// **The rule, stated as a test.** No tunnel failure may put the core's
    /// text on screen, and no sentence any of them draws may contain the
    /// machine word — which is what the person actually read before this
    /// existed.
    @Test func noTunnelFailurePutsAMachineWordOnAScreen() {
        for word in RunnerTrouble.TunnelWord.allCases {
            let kind = RunnerTrouble.tunnelFailed(word)
            let message = "cannot open the tunnel: \(word.rawValue)"
            #expect(
                !kind.showsTheRunnersOwnWords,
                "\(word.rawValue) would print the core's own words in a box")
            for line in [kind.headline(tunneled), kind.detail(message: message, words: tunneled)] {
                #expect(!line.contains(word.rawValue), "the stable word leaked into the copy: \(line)")
                #expect(!line.contains("cannot open the tunnel"), "the core's log line: \(line)")
            }
            #expect(kind.detail(message: message, words: tunneled).hasSuffix("."), "not a sentence")
        }
    }

    /// A tunneled runner has no address, so the one headline that names it has
    /// to name it the way `Runner.named` does. `words.name` is that; the empty
    /// string a tunneled runner's `address` returns would read as
    /// "Can’t Reach ".
    @Test func theTunnelHeadlineNamesTheRunnerTheWayASentenceDoes() {
        #expect(
            RunnerTrouble.tunnelFailed(.noAnswer).headline(tunneled)
                == "Can’t Reach the spare room")
        // The rendezvous is not the runner, and blaming the runner would send
        // somebody to wake a machine that was awake the whole time.
        #expect(
            RunnerTrouble.tunnelFailed(.rendezvous).headline(tunneled) == "Can’t Reach the Tunnel")
    }

    /// The one thing a person can act on when their access was revoked is
    /// knowing that it might have been. A sentence naming only a sleeping
    /// runner sends half the people who read it to the wrong place.
    @Test func theUnansweredTunnelNamesBothCauses() {
        let detail = RunnerTrouble.tunnelFailed(.noAnswer).detail(message: "", words: tunneled)
        #expect(detail.contains("asleep"))
        #expect(detail.contains("revoked"))
    }

    // MARK: - Retrying, and not

    /// **A dial cannot put the Go archive into a build that was linked without
    /// one**, so a schedule would be a ten-second timeout every thirty seconds
    /// forever for an answer that cannot change. Every other tunnel word is
    /// worth chasing: a runner that was asleep is exactly what the backoff is
    /// for.
    ///
    /// Read back here rather than left in `Connection`'s reconnect, because
    /// the iOS UI suite is compiled by CI and never executed — a `switch` in
    /// the app target is a decision nothing reads.
    @Test func theScheduleMatchesWhatTheFailureMeans() {
        for kind: RunnerTrouble in [
            .keyRejected, .hostKeyChanged, .noIdentity, .noNodeKey, .keyNotTrusted,
            .tunnelFailed(.notInThisBuild),
        ] {
            #expect(kind.retry == .never, "\(kind) must not dial again on a schedule")
        }
        #expect(RunnerTrouble.daemonMissing.retry == .afterAWhile)
        for kind: RunnerTrouble in [
            .unreachable, .stopped, .other, .tunnelFailed(.noAnswer),
            .tunnelFailed(.rendezvous), .tunnelFailed(.unspecified),
        ] {
            #expect(kind.retry == .onTheBackoff, "\(kind) should be chased on the backoff")
        }
    }

    // MARK: - The next move

    /// **The whole table, in one assertion.** Every one of these was a case in
    /// `FleetView.primaryAction` before the row existed, and the row and the
    /// phase both read this now — so a next move dropped in either screen is a
    /// next move dropped here, where it goes red.
    @Test func everyFailureKeepsTheOneMoveThatFixesIt() {
        #expect(RunnerTrouble.keyRejected.nextMove == .authorizeThisDevice)
        #expect(RunnerTrouble.hostKeyChanged.nextMove == .reviewTheNewKey)
        #expect(RunnerTrouble.keyNotTrusted.nextMove == .showTheKeyAgain)
        #expect(RunnerTrouble.noNodeKey.nextMove == .addThisDeviceAgain)
        #expect(RunnerTrouble.unreachable.nextMove == .tryAgain)
        #expect(RunnerTrouble.daemonMissing.nextMove == .tryAgain)
        #expect(RunnerTrouble.noIdentity.nextMove == .tryAgain)
        #expect(RunnerTrouble.stopped.nextMove == .tryAgain)
        #expect(RunnerTrouble.other.nextMove == .tryAgain)
        for word in RunnerTrouble.TunnelWord.allCases {
            #expect(RunnerTrouble.tunnelFailed(word).nextMove == .tryAgain)
        }
    }

    /// A key the runner has never seen is not fixed by dialing again, and a
    /// missing tunnel key cannot be: the dial would use the key that is
    /// missing, so the button could only fail, every time, forever. Android's
    /// row answers the first with "Try again" and iOS must not follow it there.
    @Test func theTwoFailuresRetryingCannotFixDoNotOfferItAsTheMove() {
        #expect(RunnerTrouble.keyRejected.nextMove != .tryAgain)
        #expect(RunnerTrouble.noNodeKey.nextMove != .tryAgain)
    }

    /// Retrying is worth a second, quieter offer under exactly one primary
    /// action. Under `tryAgain` it would appear twice; under a changed host key
    /// or an unanswered fingerprint it cannot work at all.
    @Test func retryIsAnAlternativeUnderOneFailureOnly() {
        #expect(RunnerTrouble.keyRejected.worthRetryingAsAlternative)
        for kind: RunnerTrouble in [
            .hostKeyChanged, .keyNotTrusted, .unreachable, .daemonMissing, .noIdentity,
            .noNodeKey, .stopped, .other,
        ] + RunnerTrouble.TunnelWord.allCases.map(RunnerTrouble.tunnelFailed) {
            #expect(!kind.worthRetryingAsAlternative, "\(kind) should not offer retry twice")
        }
    }

    /// A device with no key of its own has no runner to blame, and nothing in a
    /// runner editor would change the answer. Every other failure offers it,
    /// because a mistyped address is the commonest cause of most of them.
    @Test func everyFailureButOneOffersTheRunnerEditor() {
        #expect(!RunnerTrouble.noIdentity.offersEditingTheRunner)
        for kind: RunnerTrouble in [
            .keyRejected, .hostKeyChanged, .unreachable, .daemonMissing, .noNodeKey,
            .keyNotTrusted, .stopped, .other,
        ] + RunnerTrouble.TunnelWord.allCases.map(RunnerTrouble.tunnelFailed) {
            #expect(kind.offersEditingTheRunner, "\(kind) should offer the editor")
        }
    }

    /// Buttons in this app are title case.
    @Test func theMovesAreTitleCase() {
        #expect(RunnerTrouble.NextMove.authorizeThisDevice.label == "Authorize This Device")
        #expect(RunnerTrouble.NextMove.reviewTheNewKey.label == "Review the New Key")
        #expect(RunnerTrouble.NextMove.showTheKeyAgain.label == "Show the Key Again")
        #expect(RunnerTrouble.NextMove.addThisDeviceAgain.label == "Add This Device Again")
        #expect(RunnerTrouble.NextMove.tryAgain.label == "Try Again")
    }

    // MARK: - Alarm, and the transcript

    /// A color spent on everything says nothing about the one case that
    /// warrants it. `daemonMissing` is a runner nobody has run `host install`
    /// on, and `keyNotTrusted` and `stopped` are not faults at all.
    @Test func onlyAChangedHostKeyIsAlarming() {
        #expect(RunnerTrouble.hostKeyChanged.isAlarming)
        for kind: RunnerTrouble in [
            .keyRejected, .unreachable, .daemonMissing, .noIdentity, .noNodeKey,
            .keyNotTrusted, .stopped, .other,
        ] + RunnerTrouble.TunnelWord.allCases.map(RunnerTrouble.tunnelFailed) {
            #expect(!kind.isAlarming, "\(kind) should not be painted as alarming")
        }
    }

    /// The runner's own words go on screen only where the app has no diagnosis
    /// of its own. Everywhere else they would be a transcript under a sentence
    /// that already names the cause and the fix.
    @Test func onlyTheUndiagnosedFailureShowsTheRunnersOwnWords() {
        #expect(RunnerTrouble.other.showsTheRunnersOwnWords)
        for kind: RunnerTrouble in [
            .keyRejected, .hostKeyChanged, .unreachable, .daemonMissing, .noIdentity,
            .noNodeKey, .keyNotTrusted, .stopped,
        ] + RunnerTrouble.TunnelWord.allCases.map(RunnerTrouble.tunnelFailed) {
            #expect(!kind.showsTheRunnersOwnWords, "\(kind) has a diagnosis of its own")
        }
    }

    // MARK: - The sentences

    /// The four failures where the app writes the sentence itself, and none of
    /// them hands the core's log line to somebody who just wants their runner
    /// back.
    @Test func theAppWritesItsOwnSentenceWhereItKnowsWhatHappened() {
        let raw = "ssh: connect to host mini.local port 2222: Connection refused (os error 61)"
        #expect(
            RunnerTrouble.keyRejected.detail(message: raw, words: words)
                == "e@mini.local hasn’t been given this device’s key.")
        #expect(
            RunnerTrouble.daemonMissing.detail(message: raw, words: words)
                == "SSH connected, but the Far Cooler daemon didn’t answer. Install it there.")
        #expect(
            RunnerTrouble.other.detail(message: raw, words: words)
                == "The attempt to reach it didn’t finish.")
        for kind: RunnerTrouble in [.keyRejected, .daemonMissing, .unreachable, .other] {
            #expect(
                !kind.detail(message: raw, words: words).contains("os error"),
                "\(kind) must not print the core's log line as prose")
        }
    }

    /// A tunnel has no port to name and no address to have got wrong, so the
    /// two things a person can check are different ones. The nil port is the
    /// whole of how the copy tells them apart.
    @Test func anUnreachableTunnelIsNotToldToCheckAPort() {
        let direct = RunnerTrouble.unreachable.detail(message: "", words: words)
        #expect(direct.contains("port 2222"))
        let tunnel = RunnerTrouble.unreachable.detail(message: "", words: tunneled)
        #expect(!tunnel.contains("port"))
        #expect(tunnel.contains("tunnel"))
    }

    /// The two messages that must survive verbatim: a changed host key carries
    /// the two fingerprints being compared and must not be paraphrased, and the
    /// three sentences somebody wrote in `Connection` are already the app's own
    /// words.
    @Test func theMessagesWrittenToBeReadArePassedThrough() {
        let written = "The key mini.local presented is not the one Far Cooler has recorded."
        for kind: RunnerTrouble in [
            .hostKeyChanged, .noIdentity, .noNodeKey, .keyNotTrusted, .stopped,
        ] {
            #expect(kind.detail(message: written, words: words) == written)
        }
    }

    /// The one headline that names the runner names it the way the runner is
    /// named in a sentence — an address for a direct runner, a label for a
    /// tunneled one, which is `Runner.named`'s business and not this file's.
    @Test func theUnreachableHeadlineNamesTheRunner() {
        #expect(RunnerTrouble.unreachable.headline(words) == "Can’t Reach mini.local")
        #expect(RunnerTrouble.unreachable.headline(tunneled) == "Can’t Reach the spare room")
    }

    /// Headlines are title case, name what happened, and never blame the user
    /// for the two that are not faults.
    @Test func theHeadlinesSayWhatHappened() {
        #expect(RunnerTrouble.keyRejected.headline(words) == "Not Authorized Yet")
        #expect(RunnerTrouble.hostKeyChanged.headline(words) == "This Host’s Key Changed")
        #expect(RunnerTrouble.daemonMissing.headline(words) == "Far Cooler Isn’t Installed")
        #expect(RunnerTrouble.noIdentity.headline(words) == "This Device Has No Key")
        #expect(RunnerTrouble.noNodeKey.headline(words) == "This Device Has No Tunnel Key")
        #expect(RunnerTrouble.keyNotTrusted.headline(words) == "Key Not Trusted")
        #expect(RunnerTrouble.stopped.headline(words) == "Stopped Waiting")
        #expect(RunnerTrouble.other.headline(words) == "Can’t Connect")
    }

    /// Every kind has a mark, and the three that are about a key share one.
    /// Checked because a symbol name that does not exist draws nothing at all
    /// and reads as a layout bug rather than a typo.
    @Test func everyFailureHasAMark() {
        #expect(RunnerTrouble.keyRejected.symbol == "key.slash")
        #expect(RunnerTrouble.noIdentity.symbol == "key.slash")
        #expect(RunnerTrouble.noNodeKey.symbol == "key.slash")
        #expect(RunnerTrouble.hostKeyChanged.symbol == "exclamationmark.shield")
        #expect(RunnerTrouble.keyNotTrusted.symbol == "key")
        #expect(RunnerTrouble.unreachable.symbol == "network.slash")
        #expect(RunnerTrouble.daemonMissing.symbol == "square.and.arrow.down")
        #expect(RunnerTrouble.stopped.symbol == "clock")
        #expect(RunnerTrouble.tunnelFailed(.noAnswer).symbol == "network.slash")
        #expect(RunnerTrouble.tunnelFailed(.rendezvous).symbol == "network.slash")
        #expect(RunnerTrouble.tunnelFailed(.unspecified).symbol == "network.slash")
        // Not the network's mark: nothing about this device's connection
        // changes which archive a build linked.
        #expect(RunnerTrouble.tunnelFailed(.notInThisBuild).symbol == "exclamationmark.triangle")
        #expect(RunnerTrouble.other.symbol == "exclamationmark.triangle")
    }
}
