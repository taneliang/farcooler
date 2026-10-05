import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The Mac reads budgets, trends and the week from what the real CLI prints
/// (ov-307): a scratch daemon, a board with a card, a theme and a lane,
/// budgets set with `plan theme set --budget` and `plan lane set --budget`, then
/// the plan's own read, decoded by the parser the app uses. The fixture alone
/// would let the two drift (ov-160). Skipped on a Mac with no CLI build or no
/// tmux; never on CI, where the swift job builds both binaries and installs
/// tmux, so a missing one fails here rather than the test going quiet.
@MainActor
struct RealCLICostTests {
    private func farcooler(_ args: [String], home: String) async -> (ok: Bool, out: Data, err: String) {
        var environment = ProcessInfo.processInfo.environment
        environment["FARCOOLER_HOME"] = home + "/h"
        for key in ["FARCOOLER_WORKSPACE", "FARCOOLER_ACTOR", "FARCOOLER_TASK"] { environment.removeValue(forKey: key) }
        let ran = await ProcessRunner.run(RealCLIPagesTests.cli!, args, environment: environment, deadline: 60)
        return (ran.succeeded, ran.stdout, String(decoding: ran.stderr, as: UTF8.self))
    }

    @Test("plan --json from the real CLI carries the budgets set through it, and the cost a board_cost runner sends", .enabled(if: RealCLIPagesTests.runnable))
    func theRealCLIsBudgetsDecode() async throws {
        try #require(RealCLIPagesTests.cli.map { FileManager.default.isExecutableFile(atPath: $0) } == true, "no farcooler CLI in target/: build it first (CI's swift job does, in build-app.sh)")
        try #require(RealCLIPagesTests.daemon, "no farcoolerd beside the CLI")
        try #require(RealCLIPagesTests.tmux, "no tmux: CI's swift job installs it before the Mac tests")
        let home = "/tmp/fcc-\(UUID().uuidString.prefix(6))"
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
        let made = await farcooler(["--json", "workspace", "create", "demo", "--name", "Cost", "--prefix", "pc"], home: home)
        let workspace = try #require(try JSONSerialization.jsonObject(with: made.out) as? [String: Any], "\(made.err)")
        let id = try #require(workspace["id"] as? String)
        let repository = try #require(workspace["repository"] as? String ?? (workspace["repository_id"] as? String), "\(workspace)")
        let card = await farcooler(["task", "create", "--workspace", "pc", "--repo", "demo", "--title", "Round half-cents"], home: home)
        #expect(card.ok, "\(card.err)")
        let plan = ["plan", "--repo", "demo", "--workspace", "pc", "--actor", "manager"]
        for args in [
            ["theme", "create", "Money", "--outcome", "Rounding is decided.", "--card", "pc-1"],
            ["lane", "start", "rounding", "--card", "pc-1"],
            ["theme", "set", "Money", "--budget", "5M"],
            ["lane", "set", "rounding", "--budget", "800k"],
        ] {
            let ran = await farcooler(plan + args, home: home)
            #expect(ran.ok, "\(args): \(ran.err)")
        }

        let read = try PlanModel.decode(await farcooler(["plan", "--repo", repository, "--workspace", id, "--json"], home: home).out)
        let money = try #require(read.themes.first { $0.name == "Money" })
        #expect(money.budgetTokens == 5_000_000, "the theme's budget, as `--budget 5M` set it")
        #expect(money.trendTokens?.count == 7 && PlanWords.trend(money.trendTokens) == nil, "seven quiet days draw no chart")
        #expect(read.lanes.first { $0.name == "rounding" }?.budgetTokens == 800_000)
        let cost = try #require(read.cost, "a runner with board_cost sends cost")
        #expect(cost.weekTokens == 0 && cost.compare.isEmpty && !cost.isWorthShowing)
        #expect(PlanWords.overBudget(money) == nil, "nothing spent is within any budget")

        // A budget goes away again, and a refusal says why.
        #expect(await farcooler(plan + ["theme", "set", "Money", "--no-budget"], home: home).ok)
        let after = try PlanModel.decode(await farcooler(["plan", "--repo", repository, "--workspace", id, "--json"], home: home).out)
        #expect(after.themes.first { $0.name == "Money" }?.budgetTokens == nil)
        let refused = await farcooler(plan + ["theme", "set", "Money", "--budget", "lots"], home: home)
        #expect(!refused.ok && refused.err.contains("A budget is a number of tokens"), "\(refused.err)")
    }
}
