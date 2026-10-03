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

    // MARK: - Reading the core's word

    /// `test/fixtures/connect-trouble.json`: every `trouble` word the core
    /// sends on a failed connect, and every `tunnel` word, each with what the
    /// phones must make of it. Rust's `the_connect_words_are_the_shared_fixture`
    /// holds the core to the same file and `TunnelWordTest` holds Android, so
    /// a word renamed on any side fails somewhere (ov-127).
    struct ConnectWords: Decodable {
        var trouble: [String: String]
        var tunnel: [String: String]

        static func load() throws -> ConnectWords {
            var root = URL(fileURLWithPath: #filePath)
            // …/apps/shared/AgentKit/Tests/AgentKitTests/<this file>
            for _ in 0..<6 { root.deleteLastPathComponent() }
            let data = try Data(
                contentsOf: root.appendingPathComponent("test/fixtures/connect-trouble.json"))
            return try JSONDecoder().decode(ConnectWords.self, from: data)
        }
    }

    /// Every `trouble` word, from the fixture.
    static var everyCoreWord: [String] {
        (try? ConnectWords.load().trouble.keys.sorted()) ?? []
    }

    private static let kinds: [String: RunnerTrouble] = [
        "key_rejected": .keyRejected, "host_key_changed": .hostKeyChanged,
        "unreachable": .unreachable, "daemon_missing": .daemonMissing, "other": .other,
    ]
    private static let tunnelWords: [String: RunnerTrouble.TunnelWord] = [
        "no_answer": .noAnswer, "rendezvous": .rendezvous,
        "not_in_this_build": .notInThisBuild, "unspecified": .unspecified,
    ]

    /// Every word in the fixture means here what the fixture says it means.
    @Test func everyCoreWordMeansWhatTheSharedFixtureSays() throws {
        let words = try ConnectWords.load()
        #expect(words.trouble.count > 10, "the fixture lists too little to be the table")
        for (word, meaning) in words.trouble {
            switch meaning {
            case "question":
                #expect(
                    RunnerTrouble.hostKeyQuestion(trouble: word, fingerprint: "SHA256:x") != nil,
                    "\(word)")
            case "tunnel":
                for (tunnel, named) in words.tunnel {
                    let expected = try #require(Self.tunnelWords[named], "\(named)")
                    #expect(
                        RunnerTrouble(trouble: word, tunnel: tunnel) == .tunnelFailed(expected),
                        "\(word) \(tunnel)")
                }
            default:
                let expected = try #require(Self.kinds[meaning], "unknown meaning \(meaning)")
                #expect(RunnerTrouble(trouble: word) == expected, "\(word)")
                #expect(
                    RunnerTrouble.hostKeyQuestion(trouble: word, fingerprint: "SHA256:x") == nil,
                    "\(word)")
            }
        }
    }

    /// No word, and a word from a newer core, are `other`, which is the only
    /// kind that puts the core's own words on screen — the right reading of a
    /// failure this app cannot explain.
    @Test func aWordWithNoCaseIsUndiagnosed() {
        #expect(RunnerTrouble(trouble: "quic") == .other)
        #expect(RunnerTrouble(trouble: nil) == .other)
        #expect(RunnerTrouble(trouble: nil).showsTheRunnersOwnWords)
    }

    /// A tunnel failure is named by the tunnel's own stable word, which
    /// crosses as `tunnel` beside `"trouble": "tunnel"`.
    @Test(arguments: [
        ("no_answer", RunnerTrouble.TunnelWord.noAnswer),
        ("derp", RunnerTrouble.TunnelWord.rendezvous),
        ("no_tailcat", RunnerTrouble.TunnelWord.notInThisBuild),
        ("io", RunnerTrouble.TunnelWord.unspecified),
    ])
    func aTunnelFailureIsClassifiedByItsStableWord(
        tunnel: String, word: RunnerTrouble.TunnelWord
    ) {
        #expect(RunnerTrouble(trouble: "tunnel", tunnel: tunnel) == .tunnelFailed(word))
    }

    /// A tunnel word this build has never seen, or none at all, is still a
    /// sentence — `unspecified` — and never `other`, which is the raw text.
    @Test func aWordThisBuildHasNeverSeenIsStillASentence() {
        #expect(RunnerTrouble(trouble: "tunnel", tunnel: "quic") == .tunnelFailed(.unspecified))
        #expect(RunnerTrouble(trouble: "tunnel", tunnel: nil) == .tunnelFailed(.unspecified))
    }

    /// The first-contact question is the word plus the fingerprint field, and
    /// nothing else: no fingerprint, or another word, is no question.
    @Test func theHostKeyQuestionIsTheWordAndTheField() {
        #expect(
            RunnerTrouble.hostKeyQuestion(trouble: "host_key_unknown", fingerprint: "SHA256:abc")
                == "SHA256:abc")
        #expect(RunnerTrouble.hostKeyQuestion(trouble: "host_key_unknown", fingerprint: nil) == nil)
        #expect(RunnerTrouble.hostKeyQuestion(trouble: "host_key_unknown", fingerprint: "") == nil)
        #expect(
            RunnerTrouble.hostKeyQuestion(trouble: "host_key_changed", fingerprint: "SHA256:abc")
                == nil)
    }

    /// **The rule, stated as a test.** No tunnel failure may put the core's
    /// text on screen, and no sentence any of them draws may contain the
    /// machine word — which is what the person actually read before this
    /// existed.
    @Test func noTunnelFailurePutsAMachineWordOnAScreen() {
        for word in RunnerTrouble.TunnelWord.allCases {
            let kind = RunnerTrouble.tunnelFailed(word)
            // The core's log line, which `detail` must never fall back to.
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
