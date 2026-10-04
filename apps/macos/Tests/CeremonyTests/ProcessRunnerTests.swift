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

        let result = await CLI.run(["--json", "status"], executable: script, deadline: 20)

        #expect(result.ok)
        let data = try #require(result.output.data(using: .utf8))
        let body = try #require(try JSONSerialization.jsonObject(with: data) as? [String: Any])
        #expect(body["daemonVersion"] as? String == "1.2.3")
        #expect(result.errors.contains("Permanently added"))
    }

    @Test func aFailureKeepsEverythingTheCLISaid() async throws {
        let script = try Self.fakeCLI("echo progress; echo 'could not reach box' >&2; exit 3")
        defer { try? FileManager.default.removeItem(atPath: script) }

        let result = await CLI.run(["host", "install", "box"], executable: script, deadline: 20)

        #expect(!result.ok)
        #expect(result.said.contains("progress"))
        #expect(result.said.contains("could not reach box"))
    }

    @Test func enrollmentReadsItsTokenFromStdoutWhenSshWarns() async throws {
        let script = try Self.fakeCLI(
            "echo 'Warning: Permanently added x' >&2; echo '{\"connBlob\":\"tok\"}'")
        defer { try? FileManager.default.removeItem(atPath: script) }
        let result = await CLI.run(["client", "enroll"], executable: script, deadline: 20)
        #expect(Enrollment.token(in: result.output) == "tok")
    }

    private static func fakeCLI(_ body: String) throws -> String {
        let path = NSTemporaryDirectory() + "fake-cli-\(UUID().uuidString).sh"
        try "#!/bin/sh\n\(body)\n".write(toFile: path, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: path)
        return path
    }
}
