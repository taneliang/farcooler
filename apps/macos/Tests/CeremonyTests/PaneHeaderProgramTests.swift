import Foundation
import Testing

@testable import Far_Cooler

/// A pane's header names its program, never the title its session renamed it
/// to (ov-218).
struct PaneHeaderProgramTests {
    static func terminal(_ fields: String) throws -> Terminal {
        try JSONDecoder().decode(
            Terminal.self, from: Data(#"{"id":"t","short":"t","state":"running","epoch":1,\#(fields)}"#.utf8))
    }

    @Test func aClaudeThatRenamedItselfIsHeadedClaude() throws {
        let renamed = try Self.terminal(
            #""title":"Fix the login bug","preset":"Fix the login bug","program":"claude""#)
        #expect(renamed.headerName == "claude")
        // The session's title is still the pane's name everywhere else.
        #expect(renamed.label == "Fix the login bug")
    }

    @Test func aChangesPaneAndAnOlderCLI() throws {
        let changes = try Self.terminal(#""title":"","preset":"farcooler","program":"changes","paneMode":"changes""#)
        #expect(changes.headerName == "Changes")
        let older = try Self.terminal(#""title":"Terminal 3","preset":"zsh""#)
        #expect(older.headerName == "shell")
        let preset = try Self.terminal(#""title":"","preset":"x","program":"claude:opus""#)
        #expect(preset.headerName == "claude")
    }

    // MARK: - The real CLI

    private static var cli: String? {
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
            .deletingLastPathComponent().deletingLastPathComponent()
        return ["debug", "release"].map { root.appendingPathComponent("target/\($0)/farcooler").path }
            .first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    static var runnable: Bool {
        cli != nil
            && ["/opt/homebrew/bin/tmux", "/usr/local/bin/tmux", "/usr/bin/tmux"]
                .contains { FileManager.default.isExecutableFile(atPath: $0) }
    }

    private func farcooler(_ args: [String], home: String) async -> (ok: Bool, out: Data, err: String) {
        var environment = ProcessInfo.processInfo.environment
        environment["FARCOOLER_HOME"] = home + "/h"
        let ran = await ProcessRunner.run(Self.cli!, args, environment: environment, deadline: 60)
        return (ran.succeeded, ran.stdout, String(decoding: ran.stderr, as: UTF8.self))
    }

    /// The real CLI's bytes, through the Mac's own decoder: a shell whose title
    /// a program rewrote is listed under that title, and headed `shell`.
    @Test(
        "A pane whose program retitled it is headed by what was launched, through the real CLI",
        .enabled(if: PaneHeaderProgramTests.runnable))
    func theRealCLIsProgramHeadsThePane() async throws {
        let home = "/tmp/fcp-\(UUID().uuidString.prefix(6))"
        defer { try? FileManager.default.removeItem(atPath: home) }
        let demo = home + "/repos/demo"
        try FileManager.default.createDirectory(atPath: demo, withIntermediateDirectories: true)
        for args in [["init", "-q"], ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "i"]] {
            _ = await ProcessRunner.run("/usr/bin/git", ["-C", demo] + args, deadline: 30)
        }
        _ = await farcooler(["--json", "daemon", "ensure"], home: home)
        // The scratch tmux server too, which `daemon stop` leaves running:
        // `status` names its socket in its recovery line.
        func tearDown() async {
            let status = String(decoding: await farcooler(["status"], home: home).out, as: UTF8.self)
            _ = await farcooler(["daemon", "stop"], home: home)
            if let socket = status.firstMatch(of: /tmux -L (farcooler-[0-9a-f]+)/)?.1,
                let tmux = ["/opt/homebrew/bin/tmux", "/usr/local/bin/tmux", "/usr/bin/tmux"]
                    .first(where: { FileManager.default.isExecutableFile(atPath: $0) })
            {
                _ = await ProcessRunner.run(tmux, ["-L", String(socket), "kill-server"], deadline: 10)
            }
        }
        do {
            _ = await farcooler(["root", "add", home + "/repos"], home: home)
            _ = await farcooler(["repo", "register", demo], home: home)
            let made = await farcooler(["--json", "worktree", "create", "demo", "wt1", "--branch", "wt1", "--fork-only"], home: home)
            let worktree = try #require(
                (try JSONSerialization.jsonObject(with: made.out) as? [String: Any])?["short"] as? String, "\(made.err)")
            let created = await farcooler(["--json", "terminal", "create", worktree], home: home)
            let terminal = try #require(
                (try JSONSerialization.jsonObject(with: created.out) as? [String: Any])?["id"] as? String, "\(created.err)")
            // What Claude Code does to its pane: a title from its session.
            _ = await farcooler(
                ["terminal", "send", terminal, "printf '\\033]2;\\342\\234\\263 Fix the login bug\\007'; sleep 600\r"],
                home: home)
            var listed: Terminal?
            for _ in 0..<40 {
                let list = await farcooler(["--json", "worktree", "list"], home: home)
                let fleet = try JSONDecoder().decode(Fleet.self, from: list.out)
                listed = fleet.worktrees.flatMap(\.terminals).first { $0.id == terminal }
                if listed?.preset.contains("login") == true { break }
                try await Task.sleep(for: .milliseconds(500))
            }
            let pane = try #require(listed)
            #expect(pane.preset.contains("login"), "the runner never read the title: \(pane.preset)")
            #expect(pane.program == "shell", "the CLI's list doesn't say what was launched")
            #expect(pane.headerName == "shell")
        } catch {
            await tearDown()
            throw error
        }
        await tearDown()
    }
}
