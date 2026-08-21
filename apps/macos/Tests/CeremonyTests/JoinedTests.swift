import Foundation
import Testing

@testable import Far_Cooler

/// What a manifest means on the Mac RECEIVING it.
///
/// **The rule.** `CeremonyRunner.pending` means "this runner does NOT have this
/// device's key". The granting side decides it from what its writes did — see
/// ``Enrollment/Outcome/granting(_:)`` — and it arrives here as a fact about a
/// file on another machine. Nothing retries it. There is no queue, and a runner
/// a granting device could not write to traveling pending is the intended end
/// state.
///
/// So the receiving Mac has exactly one honest move: leave that runner out of
/// the list it just built, out of `~/.ssh/config`, out of the count — and name
/// it, with the one remedy that works. It used to do the opposite of all four:
/// every runner in the manifest was added, every one got a `Host` block, and
/// the screen counted them all as reachable. A runner nobody could write to
/// landed looking identical to a working one, and the person met it later as a
/// connection failure with nothing anywhere explaining why.
struct JoinedTests {
    // MARK: - The split

    /// THE bug: a pending runner is not something this Mac can access, and this
    /// fails against a `Joined` that reads the manifest without reading the flag.
    @Test func aPendingRunnerIsNotOneThisMacCanAccess() {
        let joined = Joined([runner("cosmo"), runner("box", pending: true)])

        #expect(joined.reachable.map(\.label) == ["cosmo"])
        #expect(joined.pending.map(\.label) == ["box"])
    }

    /// And the count says so. A runner the key never reached inflating this
    /// number is the screen telling somebody they have access they do not have.
    @Test func theCountIsOfRunnersThatTookTheKey() {
        let two = Joined([runner("cosmo"), runner("box"), runner("blue", pending: true)])
        #expect(two.summary == "This Mac can access 2 runners.")

        let one = Joined([runner("cosmo"), runner("box", pending: true)])
        #expect(one.summary == "This Mac can access cosmo.")
    }

    // MARK: - What the screen says about the rest

    /// Named, because the person is standing at the Mac whose list is missing
    /// them and "which ones" is the question. And told what to do, which is the
    /// only thing that works: this Mac cannot enroll itself anywhere.
    @Test func aPendingRunnerIsNamedWithTheOneRemedyThereIs() throws {
        let note = try #require(Joined([runner("cosmo"), runner("box", pending: true)]).note)

        #expect(note.contains("box"))
        #expect(!note.contains("cosmo"))
        #expect(note.contains("doesn’t have this Mac’s key yet"))
        #expect(note.contains("from a device that can reach it"))
    }

    /// It does not guess WHY. Asleep, no daemon installed, and a fence that
    /// could not be rewritten are identical from here, and a screen that guesses
    /// is how somebody ends up loosening an sshd setting that was never the
    /// problem. It promises no retry either, because nothing retries.
    @Test func theNoteNeitherGuessesACauseNorPromisesARetry() throws {
        let note = try #require(Joined([runner("box", pending: true)]).note)

        for guess in ["ssh", "sshd", "asleep", "offline", "network", "install"] {
            #expect(!note.lowercased().contains(guess))
        }
        for promise in ["will be", "automatically", "try again later", "shortly"] {
            #expect(!note.lowercased().contains(promise))
        }
    }

    @Test func severalPendingRunnersReadAsAList() throws {
        let note = try #require(
            Joined([runner("box", pending: true), runner("blue", pending: true)]).note)

        #expect(note.contains("box"))
        #expect(note.contains("blue"))
        #expect(note.contains("don’t have this Mac’s key yet"))
        #expect(note.contains("from a device that can reach them"))
    }

    /// The ordinary ceremony, where the screen has nothing extra to say.
    @Test func aReplyWhereEveryRunnerTookTheKeySaysNothingExtra() {
        let joined = Joined([runner("cosmo"), runner("box")])

        #expect(joined.note == nil)
        #expect(joined.pending.isEmpty)
        #expect(joined.headline == Joined.ready)
    }

    // MARK: - Nothing landed

    /// A phone grants one runner and only one — the live connection — so a
    /// write that fails there is a reply where EVERY runner is pending. The
    /// headline cannot be "This Mac is ready": nothing was added.
    @Test func aReplyWhereNothingLandedIsNotAReadyMac() {
        let joined = Joined([runner("box", pending: true)])

        #expect(joined.reachable.isEmpty)
        #expect(joined.headline != Joined.ready)
        // No count of nothing. The note below is the whole message.
        #expect(joined.summary == nil)
        #expect(joined.note != nil)
    }

    /// A reply carrying no runners at all is a different sentence, because the
    /// note has nothing to name and a screen with only a headline explains
    /// nothing.
    @Test func aReplyWithNoRunnersInItSaysThat() {
        let joined = Joined([])

        #expect(joined.note == nil)
        #expect(joined.summary == "The other device didn’t share any runners.")
    }

    // MARK: - Fixtures

    private func runner(_ label: String, pending: Bool = false) -> CeremonyRunner {
        CeremonyRunner(
            id: label, label: label, alias: "", address: "\(label).example", user: "e-liang",
            port: 22, host_key: "SHA256:whatever", pending: pending)
    }
}
