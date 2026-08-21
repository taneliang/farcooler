import Foundation
import Testing

@testable import Far_Cooler

struct EnrollmentTests {
    /// The regression that left a phone unable to reach the Mac it was added
    /// from: this Mac is the EMPTY target, and an empty `--runner` is not
    /// "local" to the CLI — it is an ssh destination with no name.
    @Test func thisMacIsEnrolledWithNoRunnerFlagAtAll() {
        let arguments = Enrollment.arguments(
            key: "ssh-ed25519 AAAAC3Nz", label: "iPhone", clientID: "id-1", scope: "control",
            shell: false, runner: "")

        #expect(!arguments.contains("--runner"))
        #expect(!arguments.contains(""))
    }

    @Test func anotherRunnerIsStillNamed() throws {
        let arguments = Enrollment.arguments(
            key: "ssh-ed25519 AAAAC3Nz", label: "iPhone", clientID: "id-1", scope: "control",
            shell: false, runner: "e-liang@cosmo")

        let flag = try #require(arguments.firstIndex(of: "--runner"))
        #expect(arguments[flag + 1] == "e-liang@cosmo")
    }

    /// The flag whose ABSENCE is the restricted line, on both routes.
    @Test func shellAccessIsAskedForOnlyWhenItIsWanted() {
        let restricted = Enrollment.arguments(
            key: "k", label: "l", clientID: "c", scope: "control", shell: false, runner: "")
        let plain = Enrollment.arguments(
            key: "k", label: "l", clientID: "c", scope: "host_admin", shell: true, runner: "")

        #expect(!restricted.contains("--shell-access"))
        #expect(plain.contains("--shell-access"))
    }

    /// Order matters to nobody but a reader, but the subcommand has to survive
    /// the flag going missing in front of it.
    @Test func theSubcommandSurvivesTheMissingFlag() {
        let arguments = Enrollment.arguments(
            key: "k", label: "l", clientID: "c", scope: "control", shell: false, runner: "")

        #expect(arguments.prefix(3) == ["--json", "client", "enroll"])
    }

    // MARK: - The id a device is enrolled under

    /// Two real Ed25519 public keys, the same pair
    /// `crates/client/src/ceremony.rs`'s own tests use, so the ids asserted here
    /// and the ids asserted there are the ids of the same keys.
    static let keyA =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB1iLbeqDzK4CDeUC3t+ffVPDI9Gk+sBwIZqJZW1NfS5 device-a"
    static let keyB =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDMdwe233CUbxjpEHkissIUGdCxhkTsDE/Zg7f+LB6S+ device-b"

    /// Derived from the key and therefore the same every time, which is the
    /// whole point of it: a device re-running the ceremony against a runner it
    /// is already on has to land on the id already in that fence, or the
    /// daemon's "already enrolled" arm — which compares client ids — sees a new
    /// device and the runner gains a second line for one Mac. The Mac used to
    /// mint a `UUID()` here, which is that failure once per run.
    @Test func theClientIDIsDerivedFromTheKeyAndDoesNotChange() throws {
        let first = try #require(DeviceKey.clientID(of: Self.keyA))
        let second = try #require(DeviceKey.clientID(of: Self.keyA))

        #expect(first == second)
        // The format is Rust's, asserted in `ceremony.rs` too. Named here
        // because a Swift-side "improvement" to it would be a Mac spelling one
        // device differently from the phone that added it.
        #expect(first.hasPrefix("farcooler-"))
    }

    /// **The rule that is invisible by looking.** A Mac is TWO enrolled lines
    /// and ONE client, so Key B's line carries KEY A's id — that is what lets a
    /// single `client revoke` take both of them, and what makes the removal copy
    /// about ssh, git and Zed access true.
    ///
    /// Key B derives to a DIFFERENT id, asserted first so that deriving each
    /// line's id from its own key cannot look equivalent to this. It would
    /// silently split one Mac into two clients, and nothing about the enrollment
    /// would fail at the time.
    @Test func bothOfAMacsLinesCarryKeyAsClientID() throws {
        let idA = try #require(DeviceKey.clientID(of: Self.keyA))
        let idB = try #require(DeviceKey.clientID(of: Self.keyB))
        #expect(idA != idB)

        // The two command lines `Enrollment.enroll` builds for one Mac: the
        // restricted line for Key A, the plain one for Key B, one id.
        let restricted = Enrollment.arguments(
            key: Self.keyA, label: "MacBook Air", clientID: idA, scope: "control", shell: false,
            runner: "e-liang@cosmo")
        let plain = Enrollment.arguments(
            key: Self.keyB, label: "MacBook Air", clientID: idA, scope: "host_admin", shell: true,
            runner: "e-liang@cosmo")

        #expect(clientID(in: restricted) == idA)
        #expect(clientID(in: plain) == idA)
        #expect(clientID(in: plain) != idB)
    }

    /// Text that is not a public key has no id, and the answer is nil — never a
    /// substitute. A `UUID()` fallback here is the bug this replaced, wearing a
    /// disguise: it would enroll a line under an id no `client revoke` can name,
    /// and report success.
    @Test func textThatIsNotAKeyHasNoClientID() {
        #expect(DeviceKey.clientID(of: "ssh-ed25519 AAAAC3Nz") == nil)
        #expect(DeviceKey.clientID(of: "") == nil)
    }

    /// The value `--client-id` carries, or nil when the flag is not there at
    /// all — which is itself a failure worth seeing rather than a crash.
    private func clientID(in arguments: [String]) -> String? {
        guard let flag = arguments.firstIndex(of: "--client-id"), flag + 1 < arguments.count
        else { return nil }
        return arguments[flag + 1]
    }
}
