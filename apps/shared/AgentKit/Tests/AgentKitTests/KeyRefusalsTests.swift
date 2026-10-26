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
        let suite = "KeyRefusalsTests.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suite))
        defer { defaults.removePersistentDomain(forName: suite) }

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

    @Test func otherRefusalsAndFailuresAreNot() {
        #expect(!KeyRefusals.isKeyRefusal("e@box: Permission denied (password)."))
        #expect(!KeyRefusals.isKeyRefusal("error: permission denied for control\ncode: scope-denied"))
        #expect(!KeyRefusals.isKeyRefusal("Permission denied (os error 13)"))
        #expect(!KeyRefusals.isKeyRefusal("ssh: connect to host box port 22: Connection refused"))
        #expect(!KeyRefusals.isKeyRefusal("Could not reach fc-cosmo over ssh."))
        #expect(!KeyRefusals.isKeyRefusal(""))
    }
}
