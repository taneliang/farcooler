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
}
