import AgentKit
import Foundation
import Testing

@testable import Far_Cooler

/// The command lines the Mac sends about worktrees, held to the CLI that has
/// to parse them.
///
/// The Mac drives a runner by running the bundled `farcooler`, so what these
/// calls put on the command line is the contract, and nothing checks it when
/// the app builds. A misspelled subcommand compiles, runs, exits 2, and the
/// call quietly comes back empty. That is how the composer's @-mention search
/// broke: it asked for `workspace file-search`, a command the CLI never had,
/// and every search found nothing.
///
/// So each call is made against a stub that records what it was actually
/// asked to run, and each recorded line is checked twice:
/// - against the line that call should send, so an argument in the wrong
///   place is caught as well as a wrong word;
/// - by the real CLI, with `--help` appended. clap parses the whole line
///   before it answers `--help`, so an unknown subcommand, an unknown flag or
///   a stray argument exits 2, and a line the CLI accepts exits 0. Nothing
///   reaches a daemon, so no runner is needed.
@MainActor
struct WorktreeCallsTests {
    /// Records every command line and answers the few whose output a call
    /// reads.
    @MainActor
    final class Recorder {
        var calls: [[String]] = []

        func answer(_ args: [String]) -> (data: Data?, message: String?) {
            calls.append(args)
            let words = args.filter { $0 != "--json" }
            switch Array(words.prefix(2)) {
            case ["worktree", "list"]:
                return (
                    Data(
                        #"""
                        {"runtime_healthy":true,"live_panes":0,"branch_prefix":"",
                         "worktrees":[{"id":"w-1","short":"w1","task":"fix-it","branch":"fix-it",
                                       "worktree":"/tmp/fix-it","state":"active","terminals":[]}]}
                        """#.utf8), nil
                )
            case ["worktree", "branches"]:
                return (Data(#"{"branches":[]}"#.utf8), nil)
            case ["worktree", "file-search"]:
                return (Data(#"{"paths":["src/main.rs"]}"#.utf8), nil)
            default:
                return (Data(), nil)
            }
        }
    }

    private func client(_ recorder: Recorder) -> DaemonClient {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in recorder.answer(args) }
        return client
    }

    private static let worktree = Worktree(
        id: "w-1", short: "w1", task: "fix-it", branch: "fix-it", repository: "repo",
        host: "", path: "/tmp/fix-it", state: "active", terminals: [])

    private static let billing = WorkspaceSummary(
        id: "0198f2c0-0000-7000-8000-0000000000dd", name: "Billing", taskPrefix: "bil", isMain: false,
        ordinal: 1, repository: "0198f2c0-0000-7000-8000-0000000000aa", orchestrator: nil)

    /// Each worktree call, the line it should send, and the lines it did.
    private func exercise() async -> [(call: String, expected: [String], sent: [[String]])] {
        var results: [(call: String, expected: [String], sent: [[String]])] = []
        func record(
            _ call: String, _ expected: [String], _ body: (DaemonClient) async -> Void
        ) async {
            let recorder = Recorder()
            await body(client(recorder))
            results.append((call, expected, recorder.calls))
        }

        await record("refresh", ["worktree", "list", "--json"]) { await $0.refresh() }
        await record("branches", ["worktree", "branches", "repo", "--json"]) {
            _ = await $0.branches(project: "repo")
        }
        await record("adoptBranch", ["worktree", "adopt", "repo", "feat/limits"]) {
            _ = await $0.adoptBranch(project: "repo", branch: "feat/limits", agent: "claude")
        }
        await record(
            "createWorktree",
            [
                "worktree", "create", "repo", "fix-it", "--branch", "fix-it", "--base", "HEAD",
                "--terminal", "shell",
            ]
        ) {
            _ = await $0.createWorktree(repo: "repo", task: "fix-it", branch: "fix-it", base: "HEAD")
        }
        await record("hide", ["worktree", "hide", "w1"]) { await $0.hideWorktree("w1") }
        await record("unhide", ["worktree", "unhide", "w1"]) { await $0.unhideWorktree("w1") }
        await record("reorder", ["worktree", "reorder", "w1", "w2"]) {
            await $0.reorderWorktrees(["w1", "w2"])
        }
        await record("remove", ["worktree", "remove", "w1"]) {
            _ = await $0.removeWorktree("w1", confirm: "")
        }
        await record("remove, confirmed", ["worktree", "remove", "w1", "--confirm", "fix-it"]) {
            _ = await $0.removeWorktree("w1", confirm: "fix-it")
        }
        await record("searchFiles", ["worktree", "file-search", "w1", "mai", "--json"]) {
            _ = await $0.searchFiles(in: Self.worktree, query: "mai")
        }
        await record("assign", ["worktree", "assign", "w1", "--to", Self.billing.id, "--json"]) {
            _ = await $0.assignWorktree(Self.worktree, to: Self.billing)
        }

        // The layout commands the ⌃B keys and the tile view send, each naming
        // the layout on screen, so the main checkout's row never acts on an
        // orchestrator's window tmux calls active.
        let named = ["--layout", "@2", "--json"]
        await record("zoom", ["layout", "zoom", "w1"] + named) {
            _ = await $0.zoomPane(nil, in: Self.worktree, layout: "@2")
        }
        await record("unzoom", ["layout", "zoom", "w1", "--off"] + named) {
            _ = await $0.zoomPane(nil, in: Self.worktree, off: true, layout: "@2")
        }
        await record("preset", ["layout", "preset", "w1", "tiled"] + named) {
            _ = await $0.applyPreset(.tiled, in: Self.worktree, layout: "@2")
        }
        await record("cycle", ["layout", "cycle", "w1"] + named) {
            _ = await $0.cycleLayout(Self.worktree, layout: "@2")
        }
        await record("focus next", ["layout", "focus", "w1", "--next"] + named) {
            _ = await $0.focusPane(step: "--next", in: Self.worktree, layout: "@2")
        }
        await record("focus previous", ["layout", "focus", "w1", "--prev"] + named) {
            _ = await $0.focusPane(step: "--prev", in: Self.worktree, layout: "@2")
        }
        await record(
            "split", ["layout", "split", "w1", "--side", "right", "--preset", "shell"] + named
        ) {
            _ = await $0.split(Self.worktree, beside: nil, side: .right, layout: "@2")
        }
        await record("rename", ["layout", "rename", "w1", "shells"] + named) {
            _ = await $0.renameLayout("shells", in: Self.worktree, layout: "@2")
        }
        await record("viewport", ["layout", "viewport", "w1", "100", "30"] + named) {
            _ = await $0.viewport(columns: 100, rows: 30, in: Self.worktree, layout: "@2")
        }
        // Naming none is still a line the CLI takes: tmux's active layout.
        await record("zoom, unnamed", ["layout", "zoom", "w1", "--json"]) {
            _ = await $0.zoomPane(nil, in: Self.worktree)
        }
        return results
    }

    /// The lines `createWorktree` sends, asked to claim for a workspace, on
    /// a runner that advertises `capabilities`.
    private static func madeWorktree(on capabilities: [String]) async -> [[String]] {
        let runner = StartTaskTests.Runner(capabilities: capabilities)
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in runner.answer(args) }
        _ = await client.createWorktree(
            repo: "repo", task: "fix-it", branch: "fix-it", base: "HEAD",
            workspace: "0198f2c0-0000-7000-8000-0000000000cc")
        return runner.calls
    }

    /// "New Worktree in X…" claims what it makes for the workspace it's
    /// handed, so the worktree isn't Unclaimed for good; on a runner without
    /// workspaces, which would refuse the flag, it claims nothing.
    @Test func aNewWorktreeFromTheSheetIsClaimed() async throws {
        let create = { (calls: [[String]]) in calls.first { Array($0.prefix(2)) == ["worktree", "create"] } }
        let made = try #require(create(await Self.madeWorktree(on: ["workspaces", "terminals", "workstreams"])))
        let at = try #require(made.firstIndex(of: "--workspace"), "\(made)")
        #expect(made[at + 1] == "0198f2c0-0000-7000-8000-0000000000cc", "\(made)")

        let old = try #require(create(await Self.madeWorktree(on: ["workspaces", "terminals"])))
        #expect(!old.contains("--workspace"), "\(old)")
    }

    @Test func eachWorktreeCallSendsTheWorktreeCommand() async {
        for (call, expected, sent) in await exercise() {
            #expect(sent.contains(expected), "\(call) sent \(sent)")
        }
    }

    /// A worktree dragged onto another workspace that the runner refuses
    /// stays where it was, and the banner says why in this app's words —
    /// chosen by the `code:` word, never the CLI's `error:` line. One the
    /// runner takes says nothing. Both re-read the fleet.
    @Test func aRefusedMoveSaysWhyInItsOwnWords() async {
        func refusal(_ stderr: String?) async -> (said: String?, calls: [[String]]) {
            var calls: [[String]] = []
            let client = DaemonClient(target: "", notifications: NotificationCenter())
            client.commandRunnerForTesting = { args in
                calls.append(args)
                if args.prefix(2) == ["worktree", "assign"], let stderr { return (nil, stderr) }
                return (Data(), nil)
            }
            let said = await client.assignWorktree(Self.worktree, to: Self.billing)
            return (said, calls)
        }
        let taken = await refusal(nil)
        #expect(taken.said == nil)
        #expect(taken.calls.contains { $0.prefix(2) == ["worktree", "list"] }, "no re-read: \(taken.calls)")

        let gone = await refusal("error: that worktree or workspace isn't on this runner any more\ncode: not-found")
        #expect(gone.said == "“fix it” or Billing isn’t on this runner anymore.")
        #expect(gone.calls.contains { $0.prefix(2) == ["worktree", "list"] }, "no re-read: \(gone.calls)")
        #expect(
            await refusal("error: no\ncode: capability-unsupported").said
                == "This runner’s Far Cooler is too old to move worktrees between workspaces. Update it there, then try again.")
        #expect(
            await refusal("error: no\ncode: scope-denied").said
                == "This runner lets Far Cooler see its workspaces but not change them.")
        #expect(
            await refusal("error: no\ncode: resource-conflict").said
                == "Billing changed while “fix it” was moving. Try again.")
        #expect(
            await refusal("error: no\ncode: invalid-argument").said
                == "This runner couldn’t move “fix it” to Billing as Far Cooler asked. That’s a problem in the app, not in anything you did.")
        #expect(
            await refusal("error: no workspace matching \"0198f2c0\"").said
                == "Couldn’t move “fix it” to Billing. Check that the runner is reachable, then try again.")
    }

    /// The reorder after a drag moved a worktree says its failure in our
    /// words: the reorder answers whether the runner took it, and the banner
    /// is the sentence that says the move held.
    @Test func aReorderAfterAMoveFailsInItsOwnWords() async {
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        let runner = Recorder()
        client.commandRunnerForTesting = { args -> (data: Data?, message: String?) in
            if args.prefix(2) == ["worktree", "reorder"] { return (nil, "error: no\ncode: not-found") }
            return runner.answer(args)
        }
        #expect(!(await client.reorderWorktrees(["w1", "w2"])))
        #expect(await client.reorderWorktrees(["w1"]), "one card has no order to refuse")
        #expect(
            DaemonClient.movedButNotPlaced(Self.worktree, to: Self.billing)
                == "Moved “fix it” to Billing, but couldn’t put it where you dropped it.")
    }

    /// The symptom the argv bug had, from the outside: a search that finds
    /// what the runner answered.
    @Test func anAtMentionSearchReturnsWhatTheRunnerFound() async {
        let found = await client(Recorder()).searchFiles(in: Self.worktree, query: "mai")
        #expect(found == ["src/main.rs"])
    }

    /// Every line those calls sent, worktree or not, and every line starting
    /// a task sends, parses in the CLI this tree builds.
    @Test func everyLineTheseCallsSendParsesInTheCLI() async throws {
        let cli = try #require(
            Self.cli,
            """
            No farcooler CLI to parse against. Build it first: apps/macos/build-app.sh, \
            or cargo build --bin farcooler, or set FARCOOLER_BIN.
            """)
        var lines = await exercise().flatMap(\.sent)

        // `startTask` builds its create from the runner's capabilities, so it
        // runs against a runner that has all of them.
        let runner = StartTaskTests.Runner(
            capabilities: [
                "workspaces", "terminals", "launch_prompt", "workspace_fork_only", "workstreams",
            ])
        let client = DaemonClient(target: "", notifications: NotificationCenter())
        client.commandRunnerForTesting = { args in runner.answer(args) }
        client.copyToClipboard = { _ in }
        _ = await client.startTask(
            project: "repo", description: "Fix the flaky test", name: "fix-flaky", agent: "claude",
            workspace: "0198f2c0-0000-7000-8000-0000000000dd")
        #expect(
            runner.calls.contains {
                $0.contains("create") && $0.contains("--fork-only") && $0.contains("--workspace")
            },
            "startTask made its worktree: \(runner.calls)")
        lines += runner.calls

        // "New Worktree in X…", claimed for a workspace.
        lines += await Self.madeWorktree(on: ["workspaces", "terminals", "workstreams"])

        // A board keyed by workspace, and one on a runner without them.
        let boards = Recorder()
        let reader = self.client(boards)
        _ = await reader.taskBoard(repository: "repo", workspace: "0198f2c0-0000-7000-8000-0000000000dd")
        _ = await reader.taskBoard(repository: "repo", workspace: nil)
        #expect(boards.calls.count == 2)
        lines += boards.calls

        var seen = Set<[String]>()
        for line in lines where seen.insert(line).inserted {
            let (status, stderr) = Self.parse(line, with: cli)
            #expect(status == 0, "farcooler \(line.joined(separator: " ")): \(stderr)")
        }
    }

    /// The CLI this tree builds: `FARCOOLER_BIN`, or the newer of cargo's two
    /// builds of it.
    private static var cli: String? {
        if let bin = ProcessInfo.processInfo.environment["FARCOOLER_BIN"] { return bin }
        let root = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // CeremonyTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // macos
            .deletingLastPathComponent()  // apps
            .deletingLastPathComponent()  // repo root
        let built = ["release", "debug"]
            .map { root.appendingPathComponent("target/\($0)/farcooler").path }
            .filter { FileManager.default.isExecutableFile(atPath: $0) }
        func modified(_ path: String) -> Date {
            (try? FileManager.default.attributesOfItem(atPath: path)[.modificationDate] as? Date)
                ?? .distantPast
        }
        return built.max { modified($0) < modified($1) }
    }

    /// Run `farcooler <line> --help`: 0 if the CLI accepts the line.
    ///
    /// With a home of its own, though `--help` reads none: no farcooler runs
    /// here against the real one.
    private static func parse(_ line: [String], with cli: String) -> (Int32, String) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: cli)
        process.arguments = line + ["--help"]
        var environment = ProcessInfo.processInfo.environment
        environment["FARCOOLER_HOME"] = FileManager.default.temporaryDirectory
            .appendingPathComponent("fc-worktree-calls-\(UUID().uuidString)").path
        process.environment = environment
        let err = Pipe()
        process.standardOutput = FileHandle.nullDevice
        process.standardError = err
        do { try process.run() } catch { return (-1, "\(error)") }
        let stderr = err.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        return (process.terminationStatus, String(decoding: stderr, as: UTF8.self))
    }
}
