import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// A diff that could not be read is not an empty diff (ov-155).
///
/// `changesDiff` answered every failure with `FileDiff()`, a gap-read failure
/// was filed as "too many to show", and a status reply that would not decode
/// became "Nothing changed here". The seam a test answers a diff through could
/// only say success, which is why none of it had a test. It returns a `Result`
/// now, so these can fail it.
@MainActor
struct DiffReadFailureTests {
    private static let worktreeJSON = """
        {"id":"w1","short":"w1","task":"t","branch":"feature",
        "worktree":"/tmp/w1","state":"ready","terminals":[]}
        """

    private func store(client: DaemonClient = DaemonClient(target: "")) throws -> ChangesStore {
        let ws = try JSONDecoder().decode(Worktree.self, from: Data(Self.worktreeJSON.utf8))
        let s = ChangesStore(client: client, worktree: ws)
        s.scope = .local
        return s
    }

    private static let line = DiffComputation.Line(
        id: 0, kind: .added, oldNumber: nil, newNumber: 1, text: "alpha")

    private static let failure = DiffReadFailure(message: "ssh: connect to host box port 22: Operation timed out")

    // MARK: - The client says failure

    @Test func aFailedCommandIsAFailureNotAnEmptyDiff() async {
        let client = DaemonClient(target: "")
        client.commandRunnerForTesting = { _ in (nil, "ssh: timed out") }
        let result = await client.changesDiff(worktree: "w1", path: "a.txt", scope: .local)
        guard case .failure(let why) = result else {
            Issue.record("a failed read came back as \(result)")
            return
        }
        #expect(why.message == "ssh: timed out")
    }

    @Test func aDiffWithNoHunksIsStillASuccess() async {
        let client = DaemonClient(target: "")
        client.commandRunnerForTesting = { _ in (Data(#"{"hunks":[]}"#.utf8), nil) }
        let result = await client.changesDiff(worktree: "w1", path: "a.txt", scope: .local)
        guard case .success(let diff) = result else {
            Issue.record("an answered read was called a failure")
            return
        }
        #expect(diff.lines.isEmpty)
    }

    // MARK: - The store keeps it apart

    @Test func aFailedReadFilesTheFileAsFailedAndRetryReadsItAgain() async throws {
        let s = try store()
        var answers: [Result<FileDiff, DiffReadFailure>] = [
            .failure(Self.failure), .success(FileDiff(lines: [Self.line])),
        ]
        var asked = 0
        s.diffSource = { _ in
            asked += 1
            return answers.removeFirst()
        }

        await s.ensure("a.txt")
        #expect(s.fileFailures == ["a.txt"])
        #expect(s.fileDiffs["a.txt"] == nil, "a failure is not filed as a diff of nothing")

        // Scrolling back to it does not ask again on its own; Try Again does.
        await s.ensure("a.txt")
        #expect(asked == 1)

        await s.retry("a.txt")
        #expect(asked == 2)
        #expect(s.fileFailures.isEmpty)
        #expect(s.fileDiffs["a.txt"]?.lines.count == 1)
    }

    @Test func aFailedRereadKeepsTheDiffAlreadyOnScreen() async throws {
        let s = try store()
        var next: Result<FileDiff, DiffReadFailure> = .success(FileDiff(lines: [Self.line]))
        s.diffSource = { _ in next }
        await s.ensure("a.txt")
        next = .failure(Self.failure)
        await s.read("a.txt")
        #expect(s.fileDiffs["a.txt"]?.lines.count == 1)
        #expect(s.fileFailures.isEmpty)
    }

    @Test func aGapThatCouldNotBeReadIsNotTooWide() async throws {
        let s = try store()
        var next: Result<FileDiff, DiffReadFailure> = .failure(Self.failure)
        s.diffSource = { _ in next }

        await s.open(gap: 0, of: 10, in: "a.txt")
        #expect(s.gapFailures == ["a.txt#0"])
        #expect(s.tooWide.isEmpty, "a failed read is not a refusal to render")
        #expect(!s.openGaps.contains("a.txt#0"))

        // The daemon answering with no lines IS the too-wide case.
        next = .success(FileDiff())
        await s.open(gap: 0, of: 10, in: "a.txt")
        #expect(s.tooWide == ["a.txt#0"])
        #expect(s.gapFailures.isEmpty)
    }

    @Test func aStatusReplyThatDoesNotDecodeIsAnError() async throws {
        let client = DaemonClient(target: "")
        client.commandRunnerForTesting = { _ in (Data(#"{"nothing":"useful"}"#.utf8), nil) }
        let s = try store(client: client)
        await s.load()
        #expect(s.error == ChangesStore.unreadableReply, "the pane would say Nothing changed here")
    }
}
