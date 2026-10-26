import Foundation
import Testing

@testable import Far_Cooler

/// The one process runner (ov-158) and the CLI's stream handling (ov-157).
///
/// Every child here is `/bin/sh -c`, so the tests need no built CLI and name
/// the exact behavior they pin: how much a child writes to which stream, how
/// long it sits, and when the caller gives up on it.
struct ProcessRunnerTests {
    private static let sh = "/bin/sh"

    /// A call that failed prints its reason on stderr and leaves stdout empty;
    /// a real-CLI test must report that reason, not "Unable to parse empty
    /// data" (ov-801).
    @Test func aFailedCallsAccountCarriesStderrAndHowItEnded() async throws {
        let failed = await ProcessRunner.run(Self.sh, ["-c", "echo no daemon here >&2; exit 3"])
        #expect(failed.account.contains("exited 3"))
        #expect(failed.account.contains("no daemon here"))
        let thrown = #expect(throws: RealCLIFailure.self) { try failed.jsonObject("worktree create") }
        #expect(thrown?.message.contains("worktree create: the CLI exited 3; stderr: no daemon here") == true)
        let slow = await ProcessRunner.run(Self.sh, ["-c", "sleep 30"], deadline: 0.3)
        #expect(slow.account.contains("killed at its deadline"))
        let fine = await ProcessRunner.run(Self.sh, ["-c", #"printf '{"id":"t"}'"#])
        #expect(try fine.jsonObject("terminal create")["id"] as? String == "t")
    }

    /// A pipe holds 64 KB. A child that writes 200 KB to stderr and only then
    /// writes stdout blocks in its stderr write until somebody reads it, and the
    /// old runners read stdout to EOF first. With both drained at once this
    /// finishes in milliseconds; a runner that reads one stream first never does,
    /// which the deadline here turns into a failure instead of a hung suite.
    @Test func moreThanAPipeOfStderrDoesNotDeadlock() async {
        let ran = await ProcessRunner.run(
            Self.sh,
            ["-c", "head -c 200000 /dev/zero | tr '\\0' e >&2; printf done"],
            deadline: 20)
        #expect(!ran.timedOut, "the child wedged on a full stderr pipe")
        #expect(ran.succeeded)
        #expect(ran.stderr.count == 200_000)
        #expect(String(decoding: ran.stdout, as: UTF8.self) == "done")
    }

    @Test func aChildThatNeverFinishesIsTerminatedAtTheDeadline() async {
        let started = Date()
        let ran = await ProcessRunner.run(Self.sh, ["-c", "exec sleep 30"], deadline: 0.3)
        #expect(ran.timedOut)
        #expect(!ran.succeeded)
        #expect(Date().timeIntervalSince(started) < 10)
    }

    /// SIGTERM is ignored here, so only the escalation ends it.
    @Test func aChildThatIgnoresTermIsKilledShortlyAfterTheDeadline() async {
        let started = Date()
        let ran = await ProcessRunner.run(
            Self.sh, ["-c", "trap '' TERM; exec sleep 30"], deadline: 0.3)
        #expect(ran.timedOut)
        #expect(Date().timeIntervalSince(started) < 10, "ran \(Date().timeIntervalSince(started)) s")
    }

    @Test func cancellingTheTaskTerminatesTheChild() async {
        let started = Date()
        let task = Task {
            await ProcessRunner.run(Self.sh, ["-c", "exec sleep 30"])
        }
        try? await Task.sleep(for: .milliseconds(300))
        task.cancel()
        let ran = await task.value
        #expect(ran.cancelled)
        #expect(!ran.succeeded)
        #expect(Date().timeIntervalSince(started) < 10)
    }

    @Test func aTaskCancelledBeforeItStartsNeverLaunchesTheChild() async {
        let marker = NSTemporaryDirectory() + "pr-\(UUID().uuidString)"
        let task = Task {
            // Cancelled before the first suspension point below.
            withUnsafeCurrentTask { $0?.cancel() }
            return await ProcessRunner.run(Self.sh, ["-c", "touch \(marker)"])
        }
        let ran = await task.value
        #expect(ran.cancelled)
        #expect(!FileManager.default.fileExists(atPath: marker))
    }

    @Test func standardInputReachesTheChild() async {
        let ran = await ProcessRunner.run(
            Self.sh, ["-c", "cat"], stdin: Data("a-token".utf8), deadline: 20)
        #expect(String(decoding: ran.stdout, as: UTF8.self) == "a-token")
    }

    @Test func aMissingExecutableIsALaunchFailure() async {
        let ran = await ProcessRunner.run("/nonexistent/farcooler-test", [])
        #expect(ran.launchFailure != nil)
        #expect(!ran.succeeded)
    }

    // MARK: - ov-157: stderr is not the answer

    /// A remote CLI passes ssh's stderr through. This is what a first contact
    /// looks like: a warning on stderr, a clean exit, JSON on stdout.
    @Test func anSshWarningOnStderrDoesNotBreakTheJSONOnStdout() async throws {
        let script = try Self.fakeCLI(
            """
            echo "Warning: Permanently added 'box' (ED25519) to the list of known hosts." >&2
            echo "** WARNING: connection is not using a post-quantum key exchange algorithm." >&2
            echo '{"daemonVersion":"1.2.3"}'
            """)
        defer { try? FileManager.default.removeItem(atPath: script) }

        let result = await Self.runCLI(script, ["--json", "status"])

        #expect(result.ok)
        let data = try #require(result.output.data(using: .utf8))
        let body = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        #expect(body["daemonVersion"] as? String == "1.2.3")
        #expect(result.errors.contains("Permanently added"))
    }

    @Test func aFailureKeepsEverythingTheCLISaid() async throws {
        let script = try Self.fakeCLI("echo progress; echo 'could not reach box' >&2; exit 3")
        defer { try? FileManager.default.removeItem(atPath: script) }

        let result = await Self.runCLI(script, ["host", "install", "box"])

        #expect(!result.ok)
        #expect(result.said.contains("progress"))
        #expect(result.said.contains("could not reach box"))
    }

    /// What `Enrollment`'s default writer reads its token from: stdout, with
    /// nothing of ssh's in it. Joined, the warning lines were in it too (the
    /// token scan skips them, which is why this checks the text itself).
    @Test func theTextAnEnrollmentReadsHasNoSshWarningInIt() async throws {
        let script = try Self.fakeCLI(
            "echo 'Warning: Permanently added x' >&2; echo '{\"connBlob\":\"tok\"}'")
        defer { try? FileManager.default.removeItem(atPath: script) }
        let result = await Self.runCLI(script, ["client", "enroll"])
        #expect(result.output == #"{"connBlob":"tok"}"#, "\(result.output)")
        #expect(Enrollment.token(in: result.output) == "tok")
    }

    /// Runs the stand-in as `/bin/sh script args...`, never as a program of its
    /// own. Executing a file written a moment ago waits on the system's
    /// first-run assessment of it, one at a time for the whole machine: 60
    /// such launches at once, on a machine at a 1-minute load of 30, took up
    /// to 36 s each and 21 of them passed this file's 20 s deadline, while
    /// the same 200 scripts read by `/bin/sh` took 2 s at the worst. The full
    /// Mac run launches many at once, and these three tests are the only ones
    /// here that did (ov-551).
    private static func runCLI(_ script: String, _ args: [String]) async -> CLI.Result {
        await CLI.run([script] + args, executable: "/bin/sh", deadline: 20)
    }

    private static func fakeCLI(_ body: String) throws -> String {
        let path = NSTemporaryDirectory() + "fake-cli-\(UUID().uuidString).sh"
        try "#!/bin/sh\n\(body)\n".write(toFile: path, atomically: true, encoding: .utf8)
        return path
    }
}
