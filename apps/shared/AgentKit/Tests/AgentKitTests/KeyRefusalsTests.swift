import Foundation
import Testing

@testable import AgentKit

/// A runner that refuses this device's key after letting it in before has
/// taken its access away; one refusal may pass (ov-818). See `KeyRefusals`.
struct KeyRefusalsTests {
    /// The backoff's spacing: about two, four and eight seconds apart.
    private static let t0 = Date(timeIntervalSince1970: 1_800_000_000)
    private static let dials = [0.0, 2, 6, 14, 44].map { t0.addingTimeInterval($0) }
    // MARK: - The count

    @Test func aRunnerThatNeverLetThisDeviceInIsAStrangersRefusalEveryTime() {
        var refusals = KeyRefusals(admitted: false)
        for _ in 0..<10 { #expect(refusals.refused() == .stranger) }
        #expect(refusals.refusals == 0)
    }

    @Test func refusalsInARowOverTenSecondsFromARunnerThatLetThisDeviceInAreARevoke() {
        var refusals = KeyRefusals(admitted: true)
        #expect(refusals.refused(at: Self.dials[0]) == .passing)
        #expect(refusals.refused(at: Self.dials[1]) == .passing)
        // Three, but six seconds: not yet.
        #expect(refusals.refused(at: Self.dials[2]) == .passing)
        #expect(refusals.refused(at: Self.dials[3]) == .revoked)
        #expect(refusals.refused(at: Self.dials[4]) == .revoked)
    }

    /// The Mac's reads can dial several at once: a burst is one moment.
    @Test func manyRefusalsWithinASecondAreStillPassing() {
        var refusals = KeyRefusals(admitted: true)
        for i in 0..<8 {
            #expect(refusals.refused(at: Self.t0.addingTimeInterval(Double(i) / 10)) == .passing)
        }
    }

    /// Ten seconds of two refusals is not enough either.
    @Test func twoRefusalsFarApartAreStillPassing() {
        var refusals = KeyRefusals(admitted: true)
        _ = refusals.refused(at: Self.t0)
        #expect(refusals.refused(at: Self.t0.addingTimeInterval(600)) == .passing)
    }

    @Test func aDialThatGetsInStartsTheCountAgain() {
        var refusals = KeyRefusals(admitted: true)
        _ = refusals.refused(at: Self.dials[0])
        _ = refusals.refused(at: Self.dials[1])
        _ = refusals.refused(at: Self.dials[2])
        refusals.letIn()
        #expect(refusals.refused(at: Self.dials[3]) == .passing)
        #expect(refusals.refused(at: Self.dials[4]) == .passing)
        #expect(refusals.verdict == .passing)
    }

    @Test func beingLetInIsRememberedForThatRunnerAcrossLaunches() throws {
        let defaults = try #require(ScratchDefaults.suite("KeyRefusalsTests.\(UUID().uuidString)"))

        #expect(KeyRefusals.remembered("studio", in: defaults).admitted == false)
        var refusals = KeyRefusals.remembered("studio", in: defaults)
        refusals.letIn("studio", in: defaults)
        #expect(KeyRefusals.remembered("studio", in: defaults).admitted)
        #expect(KeyRefusals.remembered("other", in: defaults).admitted == false)
    }

    // MARK: - What the phones make of a failed dial

    @Test func aStrangersRefusalIsStillNotAuthorizedYetAndNotRetried() {
        var refusals = KeyRefusals(admitted: false)
        for dial in Self.dials {
            let kind = RunnerTrouble.keyRejected.reading(&refusals, at: dial)
            #expect(kind == .keyRejected)
            #expect(kind.retry == .never)
            #expect(kind.nextMove == .authorizeThisDevice)
        }
    }

    /// The transient case: a refusal or two from a runner that let this
    /// device in is dialed again on the backoff, and a dial that gets in
    /// leaves no trace of it.
    @Test func aPassingRefusalIsRetriedOnTheBackoffAndNeverSaysRemoved() {
        var refusals = KeyRefusals(admitted: true)
        for dial in Self.dials.prefix(3) {
            let kind = RunnerTrouble.keyRejected.reading(&refusals, at: dial)
            #expect(kind == .refusedThisTime)
            #expect(kind.retry == .onTheBackoff)
        }
        refusals.letIn()
        let kind = RunnerTrouble.keyRejected.reading(&refusals, at: Self.dials[4])
        #expect(kind == .refusedThisTime)
        #expect(kind.retry == .onTheBackoff)
    }

    /// The revoked case: the fourth refusal on the backoff is the no-access screen,
    /// checked slowly, with Check Again.
    @Test func aHeldRefusalIsAccessRemovedCheckedSlowly() {
        var refusals = KeyRefusals(admitted: true)
        for dial in Self.dials.prefix(3) { _ = RunnerTrouble.keyRejected.reading(&refusals, at: dial) }
        let kind = RunnerTrouble.keyRejected.reading(&refusals, at: Self.dials[3])
        #expect(kind == .accessRemoved)
        #expect(kind.retry == .afterAWhile)
        #expect(kind.nextMove == .checkAgain)
        #expect(kind.nextMove.acts == [.dialAgain])
        // The owner's own runner, after Forget Old Key or a reset: their own
        // way back stays on offer (review S2).
        #expect(kind.alsoOffers == [.authorizeThisDevice])
        #expect(kind.offersEditingTheRunner)
        #expect(!kind.worthRetryingAsAlternative)
    }

    /// A runner that couldn't be reached says nothing about the key: it
    /// neither counts nor clears a refusal.
    @Test func aFailureThatIsNotARefusalLeavesTheCountAlone() {
        var refusals = KeyRefusals(admitted: true)
        _ = RunnerTrouble.keyRejected.reading(&refusals)
        #expect(RunnerTrouble.unreachable.reading(&refusals) == .unreachable)
        #expect(RunnerTrouble.tunnelFailed(.noAnswer).reading(&refusals) == .tunnelFailed(.noAnswer))
        #expect(refusals.refusals == 1)
    }

    // MARK: - What the runner said (ov-820)

    /// A Tailcat-only device is never refused after its revoke, only
    /// ignored: once the runner has said `released`, every failure to reach
    /// it is the no-access screen, from the first dial, checked slowly.
    @Test func aReleasedDeviceReadsEveryFailureToReachTheRunnerAsNoAccess() throws {
        let defaults = try #require(ScratchDefaults.suite("KeyRefusalsTests.\(UUID().uuidString)"))
        var refusals = KeyRefusals.remembered("studio", in: defaults)
        refusals.letIn("studio", in: defaults)
        refusals.release("studio", in: defaults)
        for gone in [RunnerTrouble.tunnelFailed(.noAnswer), .tunnelFailed(.unspecified), .unreachable, .keyRejected, .other] {
            let kind = gone.reading(&refusals)
            #expect(kind == .accessRemoved, "\(gone)")
            #expect(kind.retry == .afterAWhile)
        }
        // This device's own trouble, or a different host key, isn't the release.
        for own in [RunnerTrouble.noNodeKey, .noIdentity, .hostKeyChanged, .stopped] {
            #expect(own.reading(&refusals) == own)
        }
        // Not released: a tunnel that doesn't answer is a runner that may be asleep.
        var unsaid = KeyRefusals(admitted: true)
        #expect(RunnerTrouble.tunnelFailed(.noAnswer).reading(&unsaid) == .tunnelFailed(.noAnswer))
    }

    /// Kept for that runner across launches, and gone when a dial gets in:
    /// a device that's added back reconnects on its next slow check.
    @Test func aReleaseIsRememberedUntilADialGetsIn() throws {
        let defaults = try #require(ScratchDefaults.suite("KeyRefusalsTests.\(UUID().uuidString)"))
        var refusals = KeyRefusals.remembered("studio", in: defaults)
        refusals.release("studio", in: defaults)
        #expect(KeyRefusals.remembered("studio", in: defaults).released)
        #expect(!KeyRefusals.remembered("other", in: defaults).released)

        var relaunched = KeyRefusals.remembered("studio", in: defaults)
        relaunched.letIn("studio", in: defaults)
        #expect(!relaunched.released)
        #expect(!KeyRefusals.remembered("studio", in: defaults).released)
        #expect(RunnerTrouble.tunnelFailed(.noAnswer).reading(&relaunched) == .tunnelFailed(.noAnswer))
    }

    @Test func theNoAccessScreenSaysWhatHappenedAndWhatToDo() {
        let words = RunnerTrouble.Words(name: "studio", reachDetail: "e@studio", port: 22)
        #expect(RunnerTrouble.accessRemoved.headline(words) == RunnerPeople.noAccessTitle)
        #expect(RunnerTrouble.accessRemoved.detail(message: "Permission denied", words: words) == RunnerPeople.noAccessBody)
        #expect(RunnerPeople.noAccessBody.contains("ask the runner’s owner to add this device again"))
        // True for the owner too, whose own runner it may be.
        #expect(RunnerPeople.noAccessBody.contains("if the runner is yours, add it yourself"))
        #expect(RunnerTrouble.refusedThisTime.alsoOffers.isEmpty && RunnerTrouble.keyRejected.alsoOffers.isEmpty)
        #expect(RunnerPeople.noAccessTitle(to: "studio") == "You No Longer Have Access to studio")
        #expect(RunnerPeople.noAccessTitle(to: "") == RunnerPeople.noAccessTitle)
        #expect(RunnerTrouble.NextMove.checkAgain.label == "Check Again")
    }

    // MARK: - The Mac's reading of ssh

    /// OpenSSH's own words, as `farcooler --host` passes its stderr through.
    @Test func sshRefusingTheKeyIsAKeyRefusal() {
        #expect(KeyRefusals.isKeyRefusal("you@cosmo: Permission denied (publickey).\nCould not reach fc-cosmo over ssh."))
        #expect(KeyRefusals.isKeyRefusal("farcoolerd@studio.local: Permission denied (publickey,password)."))
        #expect(KeyRefusals.isKeyRefusal("e@box: Permission denied (password,publickey,keyboard-interactive)."))
    }

    // MARK: - Whose refusal it is, on the Mac (reviews S1 and 1)

    /// Real `ssh -v` output from OpenSSH 10.3 against a scratch sshd, whole,
    /// in `test/fixtures/ssh-refusals`. `agent-unanswered` alone is derived:
    /// `agent-refused` with the other word an agent fails with.
    static func sample(_ name: String) throws -> String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return try String(
            contentsOf: root.appendingPathComponent("test/fixtures/ssh-refusals/\(name).txt"), encoding: .utf8)
    }

    /// The keys in the samples, as the runner names them.
    static let revokedKey = "SHA256:GXr0aYtmcLgDEc2veRwu0VEX475UEN/n3nFeTP//P+o"
    static let admittedKey = "SHA256:L+FUm1ISP4h71h8/tMcyQkO6f8KqhMcOFcPru3/7eSY"
    static let lockedKey = "SHA256:UdkElBmTjsbY7q9zFNey0EjT6nH4twvAuP6kvbQXkhU"

    /// The runner refusing the key it let this device in with, from a file
    /// and from an agent.
    @Test(arguments: ["revoked", "revoked-agent-key"])
    func theRunnerRefusingItsOwnKeyIsProven(_ name: String) throws {
        let verbose = try Self.sample(name)
        #expect(KeyRefusals.isKeyRefusal(verbose))
        #expect(KeyRefusals.isProvenKeyRefusal(verbose: verbose, key: Self.revokedKey), "\(name)")
    }

    /// The same refusal proves nothing about a key that wasn't offered, or
    /// when the runner never named one.
    @Test func aRefusalOfAnotherKeyOrNoneNamedIsNotProven() throws {
        let verbose = try Self.sample("revoked")
        #expect(!KeyRefusals.isProvenKeyRefusal(verbose: verbose, key: Self.admittedKey))
        #expect(!KeyRefusals.isProvenKeyRefusal(verbose: verbose, key: ""))
    }

    /// Every other way to "Permission denied (publickey)", each asked about
    /// the key it would have had to be, and the failures that aren't refusals.
    @Test(arguments: [
        // The runner's own key refused, but with the agent missing: not
        // counted either (ov-818 review 1b). Without the agent, ssh may have
        // offered only what it found on disk, which can't be told from here.
        ("revoked-agent-missing", revokedKey),
        ("stray-key-agent-missing", revokedKey),  // no agent: a stray ~/.ssh key offered and refused
        ("stray-key-agent-empty", revokedKey),  // an agent holding nothing: the same
        ("agent-refused", admittedKey),  // the agent wouldn't sign: a locked agent, a confirm-each-use key
        ("agent-unanswered", admittedKey),  // a Touch ID or Secretive prompt nobody answered
        ("passphrase-no-agent", lockedKey),  // the server would take the key; nothing could unlock it
        ("proxyjump", revokedKey),  // a ProxyJump route, whose refusal may be the bastion's
        ("impostor", revokedKey),  // a different host key, let through by StrictHostKeyChecking no
        ("host-key-changed", revokedKey),  // strict checking stopped at the host key
        ("penalized", revokedKey),  // sshd's PerSourcePenalties turned the source away
        ("admitted", admittedKey),  // it got in
    ])
    func everyOtherWayToARefusalIsNotProven(_ name: String, _ key: String) throws {
        #expect(!KeyRefusals.isProvenKeyRefusal(verbose: try Self.sample(name), key: key), "\(name)")
    }

    /// The stray key would have been proven on the first round's rules: the
    /// host key matched, a key was offered and refused. What stops it is that
    /// it wasn't the runner's key, and that the agent wasn't there.
    @Test(arguments: ["stray-key-agent-missing", "stray-key-agent-empty"])
    func aStrayKeyLooksLikeTheRunnersRefusalInEveryOtherWay(_ name: String) throws {
        let verbose = try Self.sample(name)
        #expect(verbose.contains("is known and matches the") && verbose.contains("Offering public key"))
        #expect(KeyRefusals.isKeyRefusal(verbose))
        #expect(!verbose.contains("Server accepts key"))
    }

    /// The CLI's own stderr already says when the agent is the trouble, and
    /// then no probe is made, so no agent is asked to prompt again.
    @Test func theAgentsTroubleIsReadOffTheCLIsStderr() throws {
        let signing = """
            sign_and_send_pubkey: signing failed for ED25519 "/Users/e/.ssh/farcooler" from agent: agent refused operation
            you@cosmo: Permission denied (publickey).
            Could not reach you@cosmo over ssh.
            """
        #expect(KeyRefusals.showsAgentTrouble(signing))
        for name in ["agent-refused", "agent-unanswered", "stray-key-agent-missing", "stray-key-agent-empty"] {
            #expect(KeyRefusals.showsAgentTrouble(try Self.sample(name)), "\(name)")
        }
        #expect(!KeyRefusals.showsAgentTrouble("you@cosmo: Permission denied (publickey).\nCould not reach you@cosmo over ssh."))
        #expect(!KeyRefusals.showsAgentTrouble(try Self.sample("revoked-agent-key")), "a healthy agent is no trouble")
    }

    @Test func theKeyTheRunnerNamedIsRememberedPerRunner() throws {
        let defaults = try #require(ScratchDefaults.suite("KeyRefusalsTests.\(UUID().uuidString)"))
        var refusals = KeyRefusals.remembered("studio", in: defaults)
        #expect(refusals.key == nil)
        refusals.learn(key: Self.revokedKey, for: "studio", in: defaults)
        #expect(KeyRefusals.remembered("studio", in: defaults).key == Self.revokedKey)
        #expect(KeyRefusals.remembered("other", in: defaults).key == nil)
    }

    @Test func aProbeIsMadeAtMostEveryThirtySeconds() {
        var refusals = KeyRefusals(admitted: true)
        let first = refusals.mayProbe(at: Self.t0)
        let tooSoon = refusals.mayProbe(at: Self.t0.addingTimeInterval(29))
        let later = refusals.mayProbe(at: Self.t0.addingTimeInterval(30))
        #expect(first && !tooSoon && later)
    }

    // MARK: - Fewer failed logins on the runner's address (ov-821)

    /// A probe that proves nothing is asked twice at the usual spacing and
    /// then once in five minutes: a ProxyJump says the same however often it
    /// is asked, and each ask is a failed login for the runner's address.
    @Test func aProbeThatProvesNothingIsAskedLessAndLess() {
        var refusals = KeyRefusals(admitted: true)
        var asked: [TimeInterval] = []
        for offset in stride(from: 0.0, through: 700, by: 10) {
            guard refusals.mayProbe(at: Self.t0.addingTimeInterval(offset)) else { continue }
            asked.append(offset)
            refusals.probed(proved: false)
        }
        #expect(asked == [0, 30, 330, 630], "\(asked)")
        #expect(refusals.cannotBeProven)
    }

    @Test func aProbeThatProvedItselfOnceIsAskedAtTheUsualSpacing() {
        var refusals = KeyRefusals(admitted: true)
        refusals.probed(proved: false)
        refusals.probed(proved: false)
        #expect(refusals.cannotBeProven)
        refusals.probed(proved: true)
        #expect(!refusals.cannotBeProven)
        #expect(refusals.mayProbe(at: Self.t0))
        #expect(refusals.mayProbe(at: Self.t0.addingTimeInterval(30)))
    }

    @Test func beingLetInForgetsTheProbesThatProvedNothing() {
        var refusals = KeyRefusals(admitted: true)
        refusals.probed(proved: false)
        refusals.probed(proved: false)
        refusals.letIn()
        #expect(!refusals.cannotBeProven)
    }

    /// A device whose access is gone dials once per recheck: what the
    /// foreground and the network returning ask for waits, a tap doesn't
    /// consult this, and a recheck is five minutes on.
    @Test func aDeviceWhoseAccessIsGoneHoldsOffADialNobodyAskedFor() {
        var refusals = KeyRefusals(admitted: true)
        for dial in Self.dials.prefix(4) {
            refusals.dialing(at: dial)
            _ = refusals.refused(at: dial)
        }
        #expect(refusals.verdict == .revoked)
        let last = Self.dials[3]
        #expect(refusals.holdsOffDial(at: last.addingTimeInterval(1)))
        #expect(refusals.holdsOffDial(at: last.addingTimeInterval(299)))
        #expect(!refusals.holdsOffDial(at: last.addingTimeInterval(300)))
    }

    /// Short of settling, and for a device never let in, nothing waits: the
    /// backoff and a first connection are not what this is about.
    @Test func aDeviceThatIsNotSettledHoldsNothingOff() {
        var refusals = KeyRefusals(admitted: true)
        #expect(!refusals.holdsOffDial(at: Self.t0))
        for dial in Self.dials.prefix(3) {
            refusals.dialing(at: dial)
            _ = refusals.refused(at: dial)
        }
        #expect(refusals.verdict == .passing)
        #expect(!refusals.holdsOffDial(at: Self.dials[2].addingTimeInterval(1)))

        var stranger = KeyRefusals(admitted: false)
        stranger.dialing(at: Self.t0)
        #expect(!stranger.holdsOffDial(at: Self.t0.addingTimeInterval(1)))
    }

    /// A runner that said it released this device is settled without a
    /// single refusal, over Tailcat's silence.
    @Test func aDeviceTheRunnerReleasedHoldsOffADialToo() throws {
        let defaults = try #require(ScratchDefaults.suite("KeyRefusalsTests.\(UUID().uuidString)"))
        var refusals = KeyRefusals(admitted: true)
        refusals.release("studio", in: defaults)
        refusals.dialing(at: Self.t0)
        #expect(refusals.holdsOffDial(at: Self.t0.addingTimeInterval(60)))
        #expect(!refusals.holdsOffDial(at: Self.t0.addingTimeInterval(301)))
    }

    @Test func aDeviceLetInDialsAsItLikesAgain() {
        var refusals = KeyRefusals(admitted: true)
        for dial in Self.dials.prefix(4) {
            refusals.dialing(at: dial)
            _ = refusals.refused(at: dial)
        }
        refusals.letIn()
        #expect(!refusals.holdsOffDial(at: Self.dials[3].addingTimeInterval(1)))
    }

    /// The second key is a second refusal on the same recheck.
    @Test func aDeviceWhoseAccessIsGoneTriesOneKeyPerRecheck() throws {
        var refusals = KeyRefusals(admitted: true)
        #expect(refusals.triesTheOtherKey)
        for dial in Self.dials.prefix(3) { _ = refusals.refused(at: dial) }
        #expect(refusals.triesTheOtherKey, "not settled yet: the rekey's second key still gets its turn")
        _ = refusals.refused(at: Self.dials[3])
        #expect(!refusals.triesTheOtherKey)
        refusals.letIn()
        #expect(refusals.triesTheOtherKey)
        let defaults = try #require(ScratchDefaults.suite("KeyRefusalsTests.\(UUID().uuidString)"))
        refusals.release("studio", in: defaults)
        #expect(!refusals.triesTheOtherKey)
    }

    @Test func otherRefusalsAndFailuresAreNot() {
        #expect(!KeyRefusals.isKeyRefusal("e@box: Permission denied (password)."))
        #expect(!KeyRefusals.isKeyRefusal("error: permission denied for control\ncode: scope-denied"))
        #expect(!KeyRefusals.isKeyRefusal("Permission denied (os error 13)"))
        #expect(!KeyRefusals.isKeyRefusal("ssh: connect to host box port 22: Connection refused"))
        #expect(!KeyRefusals.isKeyRefusal("Could not reach fc-cosmo over ssh."))
        #expect(!KeyRefusals.isKeyRefusal(""))
    }
}
