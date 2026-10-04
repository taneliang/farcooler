import Combine
import Foundation
import Testing

@testable import Far_Cooler

/// A chat on screen reads its agent through one process, not five a second
/// (ov-229).
///
/// `AgentStream` used to start `farcooler terminal agent-subscribe` every
/// 200 ms for as long as a chat view was mounted, and to republish the chat
/// on every empty answer. These run the stream against a stand-in CLI, a
/// shell script that logs how it was called, and count.
@MainActor
@Suite(.serialized)
struct AgentFollowTests {
    /// A stand-in `farcooler` that appends its arguments to a log, one call
    /// per line, and answers like the real one.
    private static func standIn(refusingFollow: Bool = false) throws -> (binary: String, log: URL) {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("agent-follow-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let log = dir.appendingPathComponent("calls.log")
        let script = dir.appendingPathComponent("farcooler")
        let refuse =
            refusingFollow
            ? """
            case "$*" in *--follow*) echo "error: unexpected argument '--follow' found" >&2; exit 2;; esac
            """ : ""
        try """
            #!/bin/sh
            echo "$*" >> '\(log.path)'
            \(refuse)
            echo '{"epoch":0,"events":[]}'
            case "$*" in *--follow*) sleep 5;; esac
            """.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: script.path)
        return (script.path, log)
    }

    private static func calls(_ log: URL) -> [String] {
        ((try? String(contentsOf: log, encoding: .utf8)) ?? "").split(separator: "\n").map(String.init)
    }

    @Test func aChatReadsThroughOneProcess() async throws {
        let (binary, log) = try Self.standIn()
        let stream = AgentStream(terminal: "t1")
        stream.start(binary: binary, environment: [:])
        defer { stream.stop() }
        try await Task.sleep(for: .milliseconds(1200))

        let calls = Self.calls(log)
        #expect(calls.count == 1, "a second of a quiet chat started \(calls.count) processes")
        #expect(calls.first?.contains("--follow") == true, "\(calls)")
    }

    @Test func aCLIWithoutFollowIsPolledAsBefore() async throws {
        let (binary, log) = try Self.standIn(refusingFollow: true)
        let stream = AgentStream(terminal: "t1")
        stream.start(binary: binary, environment: [:])
        defer { stream.stop() }
        try await Task.sleep(for: .milliseconds(1200))

        let calls = Self.calls(log)
        #expect(calls.first?.contains("--follow") == true, "\(calls)")
        #expect(calls.dropFirst().count >= 2, "a CLI that refused --follow wasn't polled: \(calls)")
        #expect(calls.dropFirst().allSatisfy { !$0.contains("--follow") }, "\(calls)")
        #expect(stream.connectionError == nil, "a refused flag was shown as a broken chat")
    }

    /// An answer with nothing in it is the healthy case, and changes nothing.
    @Test func anEmptyAnswerPublishesNothing() async throws {
        let stream = AgentStream(terminal: "t1")
        stream.runnerForTesting = { _ in try JSONSerialization.data(withJSONObject: ["events": [], "epoch": 0]) }
        await stream.pump()

        var published = 0
        let watch = stream.objectWillChange.sink { _ in published += 1 }
        defer { watch.cancel() }
        await stream.pump()
        await stream.pump()
        #expect(published == 0, "two empty answers published \(published) times")
    }
}
