import Foundation
import Testing

@testable import Far_Cooler

/// The Mac's confirmation flows against what the real CLI prints (ov-160).
///
/// `removeWorktree` and `removeRoot` ask `TaskFailure.isConfirmationRequired`,
/// which reads a `code:` line, and the CLI printed none for these two commands:
/// a dirty worktree could not be removed, while every test here fed the parser a
/// line a person had typed. This starts a scratch daemon, makes a dirty
/// worktree, and gives the parser the stderr the binary really produced.
/// Skipped (and reported as skipped) where there is no CLI build or no tmux;
/// `crates/cli/tests/removing_asks_for_its_name.rs` is the same contract on the
/// Linux job, where it can't be.
struct RealCLIConfirmationTests {
    private static var cli: String? {
        if let bin = ProcessInfo.processInfo.environment["FARCOOLER_BIN"] { return bin }
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        return ["debug", "release"].map { root.appendingPathComponent("target/\($0)/farcooler").path }
            .first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    private static var tmux: Bool {
        ["/opt/homebrew/bin/tmux", "/usr/local/bin/tmux", "/usr/bin/tmux"]
            .contains { FileManager.default.isExecutableFile(atPath: $0) }
    }

    static var runnable: Bool { cli != nil && tmux }

    private func farcooler(_ args: [String], home: String) async -> (ok: Bool, out: String, err: String) {
        var environment = ProcessInfo.processInfo.environment
        ScratchDaemon.isolate(&environment, home: home + "/h")
        let ran = await ProcessRunner.run(Self.cli!, args, environment: environment, deadline: 60)
        return (ran.succeeded, String(decoding: ran.stdout, as: UTF8.self), String(decoding: ran.stderr, as: UTF8.self))
    }

    private func git(_ dir: String, _ args: [String]) async {
        _ = await ProcessRunner.run(
            "/usr/bin/git", ["-C", dir, "-c", "user.name=t", "-c", "user.email=t@t"] + args, deadline: 30)
    }

    @Test(
        "A dirty worktree and a wrong root name come back with the confirmation word",
        .enabled(if: RealCLIConfirmationTests.runnable))
    func theRealCLIPrintsTheWordTheMacReads() async throws {
        // Short: the daemon's socket lives under it.
        let home = "/tmp/fcm-\(UUID().uuidString.prefix(6))"
        // Stopped and removed on every way out, a failed expectation included:
        // `defer` can't await.
        do {
            try await scenario(home: home)
        } catch {
            await ScratchDaemon.stop(cli: Self.cli!, farcoolerHome: home + "/h")
            try? FileManager.default.removeItem(atPath: home)
            throw error
        }
        // Its tmux server too, which `daemon stop` leaves running.
        await ScratchDaemon.stop(cli: Self.cli!, farcoolerHome: home + "/h")
        try? FileManager.default.removeItem(atPath: home)
    }

    private func scenario(home: String) async throws {
        let demo = home + "/repos/demo"
        try FileManager.default.createDirectory(atPath: demo, withIntermediateDirectories: true)
        await git(demo, ["init", "-q"])
        await git(demo, ["commit", "-q", "--allow-empty", "-m", "init"])

        let ensured = await farcooler(["--json", "daemon", "ensure"], home: home)
        #expect(ensured.ok, "\(ensured.err)")
        _ = await farcooler(["root", "add", home + "/repos"], home: home)
        _ = await farcooler(["repo", "register", demo], home: home)
        let made = await farcooler(
            ["--json", "worktree", "create", "demo", "wt1", "--branch", "wt1", "--fork-only"], home: home)
        let body = try #require(
            try JSONSerialization.jsonObject(with: Data(made.out.utf8)) as? [String: Any], "\(made.err)")
        let short = try #require(body["short"] as? String)
        let path = try #require(body["worktree"] as? String)
        try "x".write(toFile: path + "/dirty.txt", atomically: true, encoding: .utf8)

        // Exactly the arguments `removeWorktree` sends for its first call.
        let removal = await farcooler(["worktree", "remove", short, "--json"], home: home)
        #expect(!removal.ok)
        #expect(TaskFailure.isConfirmationRequired(removal.err), "the Mac would call this a failure: \(removal.err)")

        // And `removeRoot` with a name that doesn't match.
        let roots = await farcooler(["root", "list"], home: home)
        let root = try #require(roots.out.split(whereSeparator: \.isWhitespace).first.map(String.init))
        let wrong = await farcooler(["root", "remove", root, "--confirm", "wrong", "--json"], home: home)
        #expect(TaskFailure.isConfirmationRequired(wrong.err), "\(wrong.err)")

        let removed = await farcooler(["worktree", "remove", short, "--confirm", "wt1", "--json"], home: home)
        #expect(removed.ok, "\(removed.err)")
    }
}
