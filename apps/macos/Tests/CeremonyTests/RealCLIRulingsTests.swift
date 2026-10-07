import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The owner's marks on a ruling, through the real CLI and a scratch daemon
/// (ov-333): the app's own Keep and Keep All arguments, the orchestrator's
/// `reverse --sha`, and the plan's own read decoded by the parser the app
/// uses. The fixture alone would let the two drift (ov-160). Skipped on a Mac
/// with no CLI build or no tmux; never on CI, where the swift job builds both
/// binaries and installs tmux.
@MainActor
struct RealCLIRulingsTests {
    private func farcooler(_ args: [String], home: String) async -> (ok: Bool, out: Data, err: String) {
        var environment = ProcessInfo.processInfo.environment
        environment["FARCOOLER_HOME"] = home + "/h"
        environment["FARCOOLER_CONFIG"] = home + "/h/config.toml"
        for key in ["FARCOOLER_WORKSPACE", "FARCOOLER_ACTOR", "FARCOOLER_TASK"] { environment.removeValue(forKey: key) }
        let ran = await ProcessRunner.run(RealCLIPagesTests.cli!, args, environment: environment, deadline: 60)
        return (ran.succeeded, ran.stdout, String(decoding: ran.stderr, as: UTF8.self))
    }

    @Test("Keep, Keep All and a reversal mark, through the real CLI, read back as the app reads them", .enabled(if: RealCLIPagesTests.runnable))
    func theRealCLIKeepsAndReverses() async throws {
        try #require(RealCLIPagesTests.cli.map { FileManager.default.isExecutableFile(atPath: $0) } == true, "no farcooler CLI in target/: build it first (CI's swift job does, in build-app.sh)")
        try #require(RealCLIPagesTests.daemon, "no farcoolerd beside the CLI")
        try #require(RealCLIPagesTests.tmux, "no tmux: CI's swift job installs it before the Mac tests")
        let home = "/tmp/fcr-\(UUID().uuidString.prefix(6))"
        do {
            try await scenario(home: home)
        } catch {
            await ScratchDaemon.stop(cli: RealCLIPagesTests.cli!, farcoolerHome: home + "/h")
            try? FileManager.default.removeItem(atPath: home)
            throw error
        }
        await ScratchDaemon.stop(cli: RealCLIPagesTests.cli!, farcoolerHome: home + "/h")
        try? FileManager.default.removeItem(atPath: home)
    }

    private func scenario(home: String) async throws {
        let demo = home + "/repos/demo"
        try FileManager.default.createDirectory(atPath: demo, withIntermediateDirectories: true)
        for args in [["init", "-q"], ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"]] {
            _ = await ProcessRunner.run("/usr/bin/git", ["-C", demo] + args, deadline: 30)
        }
        #expect(await farcooler(["--json", "daemon", "ensure"], home: home).ok)
        _ = await farcooler(["root", "add", home + "/repos"], home: home)
        _ = await farcooler(["repo", "register", demo], home: home)
        let made = await farcooler(["--json", "workspace", "create", "demo", "--name", "Rulings", "--prefix", "rl"], home: home)
        let workspace = try #require(try JSONSerialization.jsonObject(with: made.out) as? [String: Any], "\(made.err)")
        let id = try #require(workspace["id"] as? String)
        let repository = try #require(workspace["repository"] as? String ?? (workspace["repository_id"] as? String), "\(workspace)")
        let plan = ["plan", "--repo", "demo", "--workspace", "rl", "--actor", "manager"]
        for decision in ["The inbox is amber.", "The gutter is 12 points.", "Unread stays on the phones."] {
            let ran = await farcooler(
                plan + ["ruling", "add", decision, "--why", "It reads as one app.", "--reversal", "One token."], home: home)
            #expect(ran.ok, "\(decision): \(ran.err)")
        }
        func read() async throws -> PlanModel {
            try PlanModel.decode(await farcooler(["plan", "--repo", repository, "--workspace", id, "--json"], home: home).out)
        }
        let start = try await read()
        #expect(start.openRulings.map(\.short) == ["R-3", "R-2", "R-1"], "newest first, all open")
        #expect(start.pastRulings.isEmpty)

        // Keep one, with the arguments the app builds.
        let one = DaemonClient.keepArguments("R-3", repository: "demo", workspace: "rl")
        let kept = await farcooler(one, home: home)
        #expect(kept.ok, "\(kept.err)")
        let afterOne = try await read()
        #expect(afterOne.openRulings.map(\.short) == ["R-2", "R-1"])
        let r3 = try #require(afterOne.pastRulings.first)
        #expect((r3.short, r3.state, r3.settledBy) == ("R-3", .confirmed, "user"))
        #expect(r3.settledAt != nil)

        // The orchestrator can't keep for the owner.
        let refused = await farcooler(plan + ["ruling", "keep", "R-2"], home: home)
        #expect(!refused.ok && refused.err.contains("owner's call"), "\(refused.err)")

        // Keep All keeps the rest.
        let all = await farcooler(DaemonClient.keepArguments(nil, repository: "demo", workspace: "rl"), home: home)
        #expect(all.ok, "\(all.err)")
        let afterAll = try await read()
        #expect(afterAll.openRulings.isEmpty)
        #expect(Set(afterAll.pastRulings.map(\.short)) == ["R-1", "R-2", "R-3"])

        // The orchestrator reverses one and marks it with the commit.
        let reversed = await farcooler(plan + ["ruling", "reverse", "R-1", "--sha", "6E7E5618"], home: home)
        #expect(reversed.ok, "\(reversed.err)")
        let end = try await read()
        let r1 = try #require(end.rulings.first { $0.short == "R-1" })
        #expect((r1.state, r1.reversedSha, r1.settledBy) == (.reversed, "6e7e5618", "manager"))
        #expect(PlanWords.rulingSettled(r1) == "Reversed in 6e7e5618")
        #expect(end.pastRulings.first?.short == "R-1", "the most recently settled leads Past Decisions")

        // The list filters in the owner's words.
        let list = await farcooler(plan + ["ruling", "list", "--state", "kept"], home: home)
        let text = String(decoding: list.out, as: UTF8.self)
        #expect(text.contains("R-2") && text.contains("R-3") && !text.contains("R-1"), "\(text)")
    }
}
