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
        #expect(!kind.offersEditingTheRunner)
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

    @Test func theNoAccessScreenSaysWhatHappenedAndWhatToDo() {
        let words = RunnerTrouble.Words(name: "studio", reachDetail: "e@studio", port: 22)
        #expect(RunnerTrouble.accessRemoved.headline(words) == RunnerPeople.noAccessTitle)
        #expect(RunnerTrouble.accessRemoved.detail(message: "Permission denied", words: words) == RunnerPeople.noAccessBody)
        #expect(RunnerPeople.noAccessBody.contains("ask the runner’s owner to add this device again"))
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

    // MARK: - Whose refusal it is, on the Mac (review S1)

    /// Real `ssh -v` output from OpenSSH 10.3 against a scratch sshd, in
    /// `test/fixtures/ssh-refusals`, reduced to its authentication lines.
    static func sample(_ name: String) throws -> String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<6 { root.deleteLastPathComponent() }
        return try String(
            contentsOf: root.appendingPathComponent("test/fixtures/ssh-refusals/\(name).txt"), encoding: .utf8)
    }

    @Test(arguments: ["revoked", "revoked-agent-key"])
    func theRunnerRefusingAnOfferedKeyUnderAKnownHostKeyIsProven(_ name: String) throws {
        let verbose = try Self.sample(name)
        #expect(KeyRefusals.isKeyRefusal(verbose))
        #expect(KeyRefusals.isProvenKeyRefusal(verbose: verbose), "\(name)")
    }

    /// Each way to "Permission denied (publickey)" that isn't the runner
    /// refusing the key, and the two failures that aren't refusals at all.
    @Test(arguments: [
        "agent-refused",  // the agent wouldn't sign: a locked agent, a confirm-each-use key
        "agent-unanswered",  // a Touch ID or Secretive prompt nobody answered
        "passphrase-no-agent",  // the server would take the key; nothing could unlock it
        "bastion-refused",  // a ProxyJump bastion's own refusal
        "impostor",  // a different host key, let through by StrictHostKeyChecking no
        "host-key-changed",  // strict checking stopped at the host key
        "penalized",  // sshd's PerSourcePenalties turned the source away
        "admitted",  // it got in
    ])
    func everyOtherWayToARefusalIsNotProven(_ name: String) throws {
        let verbose = try Self.sample(name)
        #expect(!KeyRefusals.isProvenKeyRefusal(verbose: verbose), "\(name)")
    }

    /// The four that end in the same line the CLI shows are the ones that
    /// matter: without the probe, each would have counted.
    @Test(arguments: ["agent-refused", "agent-unanswered", "passphrase-no-agent", "bastion-refused", "impostor"])
    func theFalsePositivesAllLookLikeARefusalToTheCLI(_ name: String) throws {
        #expect(KeyRefusals.isKeyRefusal(try Self.sample(name)), "\(name)")
    }

    @Test func aProbeIsMadeAtMostEveryThirtySeconds() {
        var refusals = KeyRefusals(admitted: true)
        let first = refusals.mayProbe(at: Self.t0)
        let tooSoon = refusals.mayProbe(at: Self.t0.addingTimeInterval(29))
        let later = refusals.mayProbe(at: Self.t0.addingTimeInterval(30))
        #expect(first && !tooSoon && later)
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
