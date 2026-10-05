import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The Mac reads pages from what the real CLI prints (ov-284): a scratch
/// daemon, a board with a card and a theme, two pages published with
/// `page set`, then exactly the arguments `PlanStore.readPages` sends, decoded
/// by the parser the app uses. A fixture alone would let the two drift
/// (ov-160). Skipped on a Mac with no CLI build or no tmux; never on CI,
/// where the swift job builds both binaries (`build-app.sh`) and installs
/// tmux, so a missing one fails here rather than the test going quiet.
@MainActor
struct RealCLIPagesTests {
    nonisolated static var cli: String? {
        if let bin = ProcessInfo.processInfo.environment["FARCOOLER_BIN"] { return bin }
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<5 { root.deleteLastPathComponent() }
        return ["debug", "release"].map { root.appendingPathComponent("target/\($0)/farcooler").path }
            .first { FileManager.default.isExecutableFile(atPath: $0) }
    }

    nonisolated static var tmux: Bool {
        ["/opt/homebrew/bin/tmux", "/usr/local/bin/tmux", "/usr/bin/tmux"]
            .contains { FileManager.default.isExecutableFile(atPath: $0) }
    }

    /// The daemon the CLI starts: the one beside it, or it starts whichever
    /// is installed, which is a different build.
    nonisolated static var daemon: Bool {
        cli.map { FileManager.default.isExecutableFile(atPath: URL(fileURLWithPath: $0).deletingLastPathComponent().appendingPathComponent("farcoolerd").path) } ?? false
    }

    nonisolated static var onCI: Bool { ProcessInfo.processInfo.environment["CI"] != nil }

    nonisolated static var runnable: Bool {
        (cli.map { FileManager.default.isExecutableFile(atPath: $0) } == true && tmux && daemon) || onCI
    }

    private func farcooler(_ args: [String], home: String) async -> (ok: Bool, out: Data, err: String) {
        var environment = ProcessInfo.processInfo.environment
        environment["FARCOOLER_HOME"] = home + "/h"
        for key in ["FARCOOLER_WORKSPACE", "FARCOOLER_ACTOR", "FARCOOLER_TASK"] { environment.removeValue(forKey: key) }
        let ran = await ProcessRunner.run(Self.cli!, args, environment: environment, deadline: 60)
        return (ran.succeeded, ran.stdout, String(decoding: ran.stderr, as: UTF8.self))
    }

    @Test("page list --json from the real CLI decodes into the pages the Plan view lists and anchors", .enabled(if: RealCLIPagesTests.runnable))
    func theRealCLIsPagesDecode() async throws {
        try #require(Self.cli.map { FileManager.default.isExecutableFile(atPath: $0) } == true, "no farcooler CLI in target/: build it first (CI's swift job does, in build-app.sh)")
        try #require(Self.daemon, "no farcoolerd beside the CLI")
        try #require(Self.tmux, "no tmux: CI's swift job installs it before the Mac tests")
        let home = "/tmp/fcp-\(UUID().uuidString.prefix(6))"
        do {
            try await scenario(home: home)
        } catch {
            await ScratchDaemon.stop(cli: Self.cli!, farcoolerHome: home + "/h")
            try? FileManager.default.removeItem(atPath: home)
            throw error
        }
        await ScratchDaemon.stop(cli: Self.cli!, farcoolerHome: home + "/h")
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
        let made = await farcooler(["--json", "workspace", "create", "demo", "--name", "Pages", "--prefix", "pg"], home: home)
        let workspace = try #require(try JSONSerialization.jsonObject(with: made.out) as? [String: Any], "\(made.err)")
        let id = try #require(workspace["id"] as? String)
        let repository = try #require(workspace["repository"] as? String ?? (workspace["repository_id"] as? String), "\(workspace)")
        let card = await farcooler(["task", "create", "--workspace", "pg", "--repo", "demo", "--title", "Round half-cents"], home: home)
        #expect(card.ok, "\(card.err)")
        let theme = await farcooler(
            ["plan", "--repo", "demo", "--workspace", "pg", "--actor", "manager", "theme", "create", "Money", "--outcome", "Rounding is decided.", "--card", "pg-1"], home: home)
        #expect(theme.ok, "\(theme.err)")

        let own = home + "/own.json", risks = home + "/risks.json"
        try #"{"v":1,"title":"Own","summary":"One of its own","blocks":[{"type":"list","items":[{"text":"Rounding","state":"waiting","ref":{"task":"pg-1"}}]},{"type":"links","items":[{"theme":"Money"},{"url":"https://github.com/example"}]}]}"#
            .write(toFile: own, atomically: true, encoding: .utf8)
        try #"{"v":1,"title":"Risks","blocks":[{"type":"heading","text":"Risks"},{"type":"timeline","entries":[{"at":"2026-10-04T15:02:00-07:00","text":"Picked"}]}]}"#
            .write(toFile: risks, atomically: true, encoding: .utf8)
        for (slot, file, extra) in [("own", own, [String]()), ("risks", risks, ["--theme", "Money"])] {
            let set = await farcooler(
                ["page", "--repo", "demo", "--workspace", "pg", "--actor", "manager", "set", slot, "--file", file] + extra, home: home)
            #expect(set.ok, "\(set.err)")
        }

        // What the app sends, with the ids it holds.
        let listed = await farcooler(DaemonClient.pageListArguments(repository: repository, workspace: id), home: home)
        #expect(listed.ok, "\(listed.err)")
        let pages = try BoardPageList.decode(listed.out).pages
        #expect(pages.map(\.slot).sorted() == ["own", "risks"])
        let ownPage = try #require(pages.first { $0.slot == "own" })
        #expect(ownPage.title == "Own" && ownPage.summary == "One of its own" && ownPage.actor == "manager" && ownPage.revision == 1)
        #expect(ownPage.updatedAtMs > 0 && ownPage.themeAnchor == nil)
        guard case .list(let items)? = ownPage.doc?.blocks.first else {
            Issue.record("the list didn't decode: \(String(describing: ownPage.doc))")
            return
        }
        #expect(items.first?.ref == PageRef(.task("pg-1")) && items.first?.state == .waiting)
        let risksPage = try #require(pages.first { $0.slot == "risks" })
        guard case .timeline(let entries, _)? = risksPage.doc?.blocks.last else {
            Issue.record("the timeline didn't decode")
            return
        }
        #expect(entries.first?.at == 1_791_151_320_000, "the runner stores milliseconds")

        // And the plan's own read, so the anchor lands on its theme.
        let planRead = await farcooler(["plan", "--repo", repository, "--workspace", id, "--json"], home: home)
        let plan = try PlanModel.decode(planRead.out)
        let money = try #require(plan.themes.first { $0.name == "Money" })
        #expect(risksPage.themeAnchor == money.id)
        #expect(PlanStore.listed(pages, plan: plan, hidden: []).map(\.slot) == ["own"])
        #expect(PlanStore.anchored(pages, to: money.id, plan: plan, hidden: []).map(\.slot) == ["risks"])
    }
}
