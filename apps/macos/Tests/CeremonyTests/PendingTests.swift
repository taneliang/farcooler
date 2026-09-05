import Foundation
import Testing

@testable import Far_Cooler

/// What `pending` says about a runner, and whether it is true.
///
/// **The rule.** `CeremonyRunner.pending` means "this runner does NOT yet have
/// this device's key". A device that receives a record with `pending: false`
/// believes its key is in that runner's `~/.ssh/authorized_keys` and that it
/// can connect. So a false there is a promise about a file on another machine,
/// and the only thing on this Mac that knows whether that promise holds is the
/// enrollment that just ran.
///
/// It used to be hardcoded false in ``RunnerFacts`` — on every runner, before
/// anything was written — so a Mac that could not reach a runner told the new
/// device it was ready anyway. The new device then failed to connect with
/// nothing on either screen explaining why, which is the one failure in this
/// flow a person cannot diagnose.
///
/// These tests drive ``Enrollment/enroll(keyA:keyB:label:clientID:scope:on:using:)``
/// with a stand-in for the CLI, because the decision being tested is made from
/// what each `client enroll` answered and nothing else.
struct PendingTests {
    // MARK: - The records the new device is handed

    /// A runner the write failed on travels pending. THE bug: this fails
    /// against a `pending` that is decided anywhere but here.
    @Test func aRunnerThatCouldNotBeWrittenToTravelsPending() async throws {
        let runners = [runner("e-liang@cosmo"), runner("e-liang@box")]

        let outcome = await Enrollment.enroll(
            keyA: Self.keyA, keyB: nil, label: "iPhone 17", clientID: "farcooler-1",
            scope: "control", on: runners,
            using: answering { target, _ in target != "e-liang@box" })

        let granting = outcome.granting(runners)
        #expect(granting.first { $0.id == "e-liang@box" }?.pending == true)
        // And the reason is still in the CLI's own words, for the box under it.
        #expect(outcome.transcript?.contains("e-liang@box") == true)
    }

    /// A runner that took the key does not, which is the half that makes the
    /// flag worth carrying at all.
    @Test func aRunnerThatTookTheKeyDoesNotTravelPending() async throws {
        let runners = [runner("e-liang@cosmo"), runner("e-liang@box")]

        let outcome = await Enrollment.enroll(
            keyA: Self.keyA, keyB: nil, label: "iPhone 17", clientID: "farcooler-1",
            scope: "control", on: runners,
            using: answering { target, _ in target != "e-liang@box" })

        let granting = outcome.granting(runners)
        #expect(granting.first { $0.id == "e-liang@cosmo" }?.pending == false)
        #expect(Enrollment.note(about: granting, outcome: outcome) == Enrollment.couldNotReachAll)
    }

    /// Every runner takes it, nothing is pending, and the screen says nothing.
    @Test func aCeremonyWhereEverythingLandedSaysNothing() async throws {
        let runners = [runner("e-liang@cosmo"), runner("e-liang@box")]

        let outcome = await Enrollment.enroll(
            keyA: Self.keyA, keyB: nil, label: "iPhone 17", clientID: "farcooler-1",
            scope: "control", on: runners, using: answering { _, _ in true })

        let granting = outcome.granting(runners)
        #expect(granting.allSatisfy { !$0.pending })
        #expect(outcome.transcript == nil)
        #expect(Enrollment.note(about: granting, outcome: outcome) == nil)
    }

    /// A runner that was never attempted is pending too — no entry in the
    /// outcome is the same answer as a failed one, because the file is in the
    /// same state either way.
    ///
    /// This is the shape of the unreadable-key branch in
    /// `AddDeviceView.confirm(name:)`: no client id means nothing to enroll
    /// under, so nothing is written and every runner travels pending. iOS takes
    /// the same branch and leaves every runner pending as well.
    @Test func nothingEnrolledLeavesEveryRunnerPending() {
        let runners = [runner("e-liang@cosmo"), runner(""), runner("e-liang@box")]

        let outcome = Enrollment.Outcome.nothingWritten(Enrollment.unreadableKey)
        let granting = outcome.granting(runners)

        #expect(granting.allSatisfy { $0.pending })
        #expect(Enrollment.note(about: granting, outcome: outcome) == Enrollment.couldNotReachAll)
    }

    // MARK: - This Mac

    /// The local daemon is a runner like the others, and starts pending like
    /// the others.
    ///
    /// Reaching a Unix socket is not the same as having written to
    /// `~/.ssh/authorized_keys` here: that write is a `client enroll` with no
    /// `--runner`, and it fails when the daemon is missing, stopped, or
    /// refuses. A hardcoded false was how a phone got told its key had landed
    /// on the very Mac it was being added from.
    @Test func thisMacStartsPendingLikeEveryOtherRunner() {
        #expect(RunnerFacts.thisMac().pending)
    }

    /// And loses the flag exactly when the local write answers, which is the
    /// reason it is not exempt rather than merely pessimistic. The empty target
    /// is how this whole app names this Mac, so it carries no `--runner` at
    /// all.
    @Test func thisMacIsClearedByItsOwnWriteAndNotByBeingLocal() async throws {
        let runners = [runner("")]

        let landed = await Enrollment.enroll(
            keyA: Self.keyA, keyB: nil, label: "iPhone 17", clientID: "farcooler-1",
            scope: "control", on: runners,
            using: answering { target, _ in target.isEmpty })
        #expect(landed.granting(runners).allSatisfy { !$0.pending })

        let refused = await Enrollment.enroll(
            keyA: Self.keyA, keyB: nil, label: "iPhone 17", clientID: "farcooler-1",
            scope: "control", on: runners, using: answering { _, _ in false })
        #expect(refused.granting(runners).allSatisfy { $0.pending })
    }

    // MARK: - The Mac's second key

    /// A runner that took Key A and refused Key B is NOT pending.
    ///
    /// `pending` is about the key the new device connects to Far Cooler with,
    /// which is Key A's restricted line. Key B is Zed, Git and Terminal on a
    /// Mac; marking that runner pending would tell the new Mac to ignore a
    /// runner it can talk to perfectly well. It gets its own sentence instead.
    @Test func aRunnerThatRefusedShellAccessIsReachableAndSaysSo() async throws {
        let runners = [runner("e-liang@cosmo")]

        let outcome = await Enrollment.enroll(
            keyA: Self.keyA, keyB: Self.keyB, label: "MacBook Air", clientID: "farcooler-1",
            scope: "control", on: runners, using: answering { _, shell in !shell })

        let granting = outcome.granting(runners)
        #expect(granting.allSatisfy { !$0.pending })
        #expect(outcome.shellRefused == ["e-liang@cosmo"])
        // The transcript names which half failed, and the sentence above it is
        // the one about editors rather than the one about missing runners.
        #expect(outcome.transcript?.contains("shell access") == true)
        #expect(
            Enrollment.note(about: granting, outcome: outcome) == Enrollment.shellAccessIncomplete)
    }

    /// Key A failing takes the whole runner with it, Key B included — the
    /// runner is pending, and the pending sentence is the one that wins.
    @Test func aRunnerKeyAMissedIsPendingWhateverKeyBWouldHaveDone() async throws {
        let runners = [runner("e-liang@cosmo")]

        let outcome = await Enrollment.enroll(
            keyA: Self.keyA, keyB: Self.keyB, label: "MacBook Air", clientID: "farcooler-1",
            scope: "control", on: runners, using: answering { _, _ in false })

        let granting = outcome.granting(runners)
        #expect(granting.allSatisfy { $0.pending })
        #expect(outcome.shellRefused.isEmpty)
        #expect(Enrollment.note(about: granting, outcome: outcome) == Enrollment.couldNotReachAll)
    }

    // MARK: - Fixtures

    static let keyA = EnrollmentTests.keyA
    static let keyB = EnrollmentTests.keyB

    /// One runner, as ``RunnerFacts`` would have made it: pending, because
    /// nothing has been written to it yet.
    private func runner(_ id: String) -> CeremonyRunner {
        CeremonyRunner(
            id: id, label: id.isEmpty ? "This Mac" : id, alias: "", user: "e-liang",
            host_key: "SHA256:whatever",
            reach: .direct(host: id.isEmpty ? "cosmo.local" : "\(id).example", port: 22),
            pending: true)
    }

    /// A stand-in for `farcooler client enroll`, answering per runner and per
    /// key.
    ///
    /// It reads the target back out of the command line rather than being told
    /// which call this is, so a runner naming bug shows up here as the wrong
    /// answer instead of being papered over — an absent `--runner` is this Mac,
    /// which is the empty id everywhere else in the app.
    private func answering(
        _ decide: @escaping @Sendable (_ target: String, _ shell: Bool) -> Bool
    ) -> Enrollment.Writer {
        { arguments in
            var target = ""
            if let flag = arguments.firstIndex(of: "--runner"), flag + 1 < arguments.count {
                target = arguments[flag + 1]
            }
            let shell = arguments.contains("--shell-access")
            guard decide(target, shell) else {
                return (false, "Could not reach \(target) over ssh.")
            }
            return (true, "{\"enrolled\":true}")
        }
    }
}
