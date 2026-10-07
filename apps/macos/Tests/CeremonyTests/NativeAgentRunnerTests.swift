import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The native view against a real runner (ov-372): the real `farcooler` CLI
/// turns the setting on, a real `farcoolerd` folds a claude pane's
/// transcript, and the app's own client core (`RunnerCore`, the FFI the app
/// links) reads its rows over the daemon's socket into an `AgentRowStore`,
/// follows a record appended to it, and sends through `terminal.compose`.
/// Real bytes through the real parser, end to end.
///
/// The pane runs a stand-in, never claude (`FARCOOLER_TEST_STUB_AGENTS`),
/// and the daemon reads only this test's own CLAUDE_CONFIG_DIR and
/// config.toml.
@MainActor
@Suite(.serialized)
struct NativeAgentRunnerTests {
    nonisolated private static var cli: String? {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        return ["debug", "release"].map { root.appendingPathComponent("target/\($0)/farcooler").path }
            .first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    /// The CLI these tests run, for the one that streams a live terminal.
    nonisolated static var cliPath: String? { cli }

    nonisolated static var runnable: Bool {
        cli != nil
            && ["/opt/homebrew/bin/tmux", "/usr/local/bin/tmux", "/usr/bin/tmux"]
                .contains { FileManager.default.isExecutableFile(atPath: $0) }
    }

    private static func environment(_ home: String) -> [String: String] {
        var environment = ProcessInfo.processInfo.environment
        environment["FARCOOLER_HOME"] = home + "/h"
        environment["FARCOOLER_TEST_STUB_AGENTS"] = "1"
        environment["CLAUDE_CONFIG_DIR"] = home + "/claude"
        environment["FARCOOLER_CONFIG"] = home + "/config.toml"
        environment.removeValue(forKey: "FARCOOLER_PROJECTOR")
        return environment
    }

    private func farcooler(_ args: [String], home: String) async -> (ok: Bool, out: Data, err: String) {
        let ran = await ProcessRunner.run(Self.cli!, args, environment: Self.environment(home), deadline: 60)
        return (ran.succeeded, ran.stdout, String(decoding: ran.stderr, as: UTF8.self))
    }

    private static func append(_ line: [String: Any], to path: String) throws {
        let data = try JSONSerialization.data(withJSONObject: line) + Data("\n".utf8)
        if let handle = FileHandle(forWritingAtPath: path) {
            handle.seekToEndOfFile()
            handle.write(data)
            try handle.close()
        } else {
            try data.write(to: URL(fileURLWithPath: path))
        }
    }

    @Test(
        "The setting, a page, a follow and a send, through the real CLI, daemon and client core",
        .enabled(if: NativeAgentRunnerTests.runnable))
    func theNativeViewReadsARealRunner() async throws {
        let home = "/tmp/fcn-\(UUID().uuidString.prefix(6))"
        defer { try? FileManager.default.removeItem(atPath: home) }
        let demo = home + "/repos/demo"
        try FileManager.default.createDirectory(atPath: demo, withIntermediateDirectories: true)
        for args in [["init", "-q"], ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "i"]] {
            _ = await ProcessRunner.run("/usr/bin/git", ["-C", demo] + args, deadline: 30)
        }
        _ = await farcooler(["--json", "daemon", "ensure"], home: home)
        // Stopped however the steps below end.
        do {
            try await exercise(home: home, demo: demo)
        } catch {
            Issue.record("\(error)")
        }
        await ScratchDaemon.stop(cli: Self.cli!, farcoolerHome: home + "/h")
    }

    private func exercise(home: String, demo: String) async throws {
        let core = RunnerCore()
        let socket = RunnerCore.localSocket(environment: Self.environment(home))
        do {
            // Off until the setting turns it on, which needs no restart.
            #expect(!(try await core.connect(socket: socket)).contains("agent_rows"))
            let set = await farcooler(["settings", "set-projector", "on"], home: home)
            #expect(set.ok, "\(set.err)")
            #expect((try? String(contentsOfFile: home + "/config.toml", encoding: .utf8))?.contains("projector = true") == true)
            let fresh = RunnerCore()
            #expect(try await fresh.connect(socket: socket).contains("agent_rows"))

            _ = await farcooler(["root", "add", home + "/repos"], home: home)
            _ = await farcooler(["repo", "register", demo], home: home)
            let made = await farcooler(["--json", "worktree", "create", "demo", "wt1", "--branch", "wt1", "--fork-only"], home: home)
            let worktree = try #require(
                (try JSONSerialization.jsonObject(with: made.out) as? [String: Any])?["short"] as? String, "\(made.err)")
            let created = await farcooler(["--json", "terminal", "create", worktree, "--preset", "claude"], home: home)
            let terminal = try #require(
                (try JSONSerialization.jsonObject(with: created.out) as? [String: Any])?["id"] as? String, "\(created.err)")

            // Where claude would write the pane's transcript.
            let listed = await farcooler(["worktree", "list", "--json"], home: home)
            let worktrees = (try JSONSerialization.jsonObject(with: listed.out) as? [String: Any])?["worktrees"] as? [[String: Any]] ?? []
            let mine = try #require(worktrees.first { ($0["terminals"] as? [[String: Any]])?.contains { $0["id"] as? String == terminal } == true })
            let session = try #require((mine["terminals"] as? [[String: Any]])?.first { $0["id"] as? String == terminal }?["agentSessionId"] as? String)
            let path = try #require(mine["worktree"] as? String)
            let real = URL(fileURLWithPath: path).resolvingSymlinksInPath().path
            let project = home + "/claude/projects/" + String(real.map { $0.isASCII && ($0.isLetter || $0.isNumber || $0 == "-") ? $0 : "-" })
            try FileManager.default.createDirectory(atPath: project, withIntermediateDirectories: true)
            let transcript = project + "/\(session).jsonl"
            try Self.append(
                ["type": "user", "promptId": "p1", "promptSource": "typed", "uuid": "u-p1", "timestamp": "2026-10-06T10:00:00Z",
                 "message": ["content": "Tidy the parser."]], to: transcript)

            // A page, through the client core, into the store.
            let store = AgentRowStore(key: "runner-\(terminal)", cache: nil)
            store.start(CoreRowSource(core: fresh, terminal: terminal))
            defer { store.stop() }
            try await Self.wait("the first page") { store.ids.contains("turn:p1") }
            guard case .turn(let turn)? = store.box("turn:p1")?.row.kind else {
                Issue.record("no turn row")
                return
            }
            #expect(turn.prompt == "Tidy the parser.")
            #expect(store.phase == .live)

            // A record appended: the follow brings it, the turn row in place.
            let turnBox = store.box("turn:p1")
            try Self.append(
                ["type": "assistant", "uuid": "a1", "timestamp": "2026-10-06T10:00:03Z",
                 "message": ["content": [["type": "text", "text": "Tidied."]], "stop_reason": "end_turn"]], to: transcript)
            try await Self.wait("the reply") {
                store.ids.contains { id in
                    if case .prose(let prose)? = store.box(id)?.row.kind { return prose.text == "Tidied." }
                    return false
                }
            }
            #expect(store.box("turn:p1") === turnBox, "the turn row changed in place")

            // A send goes over the wire and comes back with the runner's word
            // for why a stand-in pane can't be typed into.
            do {
                _ = try await fresh.compose(terminal: terminal, text: "and the docs")
                Issue.record("a stand-in pane took a message")
            } catch let failure as RunnerCore.Failure {
                print("NATIVE-RUNNER compose refused: \(failure)")
                #expect(failure.what != nil, "\(failure)")
                if case .said(let words) = NativePaneModel.issue(for: failure) {
                    #expect(words != "The message wasn’t sent.", "a word the composer has no sentence for: \(failure)")
                }
            }
        }
    }

    static func wait(_ what: String, _ condition: () -> Bool) async throws {
        let deadline = ContinuousClock.now + .seconds(15)
        while ContinuousClock.now < deadline {
            if condition() { return }
            try await Task.sleep(for: .milliseconds(20))
        }
        Issue.record("timed out waiting for \(what)")
    }
}
