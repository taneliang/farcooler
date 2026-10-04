import AgentKit
import Combine
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
        /// What `status` says this runner can do, or nil to answer nothing.
        var capabilities: [String]?
        /// What `terminal agent-answer` fails with, or nil to take it.
        var answerRefusal: String?
        /// Whether `needs-you` fails, as a runner that can't be reached does.
        var needsYouFails = false
        /// Whether `needs-you` answers with nothing in it.
        var needsYouEmpty = false
        /// The extra read-only folders `status` lists (ov-232), by name.
        var folders: [String]?

        func answer(_ args: [String]) -> (data: Data?, message: String?) {
            calls.append(args)
            let words = args.filter { $0 != "--json" }
            if words == ["status"], let capabilities {
                var body: [String: Any] = [
                    "daemonVersion": "0.1.0", "buildsMatch": true, "platform": "macos",
                    "capabilities": capabilities,
                ]
                if let folders { body["readOnlyFolders"] = folders.map { ["name": $0, "path": ""] } }
                return (try? JSONSerialization.data(withJSONObject: body), nil)
            }
            if words.first == "needs-you", needsYouFails {
                return (nil, "ssh: connect to host runner: Connection refused")
            }
            if words.first == "needs-you", needsYouEmpty {
                return (Data(#"{"items":[]}"#.utf8), nil)
            }
            if words.first == "needs-you" {
                return (
                    Data(
                        #"""
                        {"items":[{"id":"decision:t-9","kind":"decision","also":[],"rank":300,
                          "since":null,"workspace_id":"ws-1","workspace_name":"Billing",
                          "repository_id":"r-1","task":{"id":"t-9","key":"bil-9",
                          "title":"Invoice PDF export","status":"needs_decision"},
                          "terminal":null,"worktree":null,"question":"Postgres or SQLite?",
                          "detail":null,"ask_id":null,"actions":[]}]}
                        """#.utf8), nil
                )
            }
            if Array(words.prefix(2)) == ["terminal", "agent-answer"], let answerRefusal {
                return (nil, answerRefusal)
            }
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
        await record("adoptBranch", ["worktree", "adopt", "repo", "feat/limits", "--json"]) {
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
        await record("hide", ["worktree", "hide", "w1", "--json"]) { await $0.hideWorktree("w1") }
        await record("unhide", ["worktree", "unhide", "w1", "--json"]) { await $0.unhideWorktree("w1") }
        // Try Again on a worktree's large files (ov-199).
        await record("hydrateLfs", ["worktree", "hydrate-lfs", "w1", "--json"]) { _ = await $0.hydrateLfs("w1") }
        await record("remove", ["worktree", "remove", "w1", "--json"]) {
            _ = await $0.removeWorktree("w1", confirm: "")
        }
        await record("remove, confirmed", ["worktree", "remove", "w1", "--confirm", "fix-it", "--json"]) {
            _ = await $0.removeWorktree("w1", confirm: "fix-it")
        }
        await record("searchFiles", ["worktree", "file-search", "w1", "mai", "--json"]) {
            _ = await $0.searchFiles(in: Self.worktree, query: "mai")
        }
        // The Files tab's two reads (ov-189), the root and a file.
        await record("listFiles", ["files", "ls", "--json", "--", "w1", ""]) {
            _ = await $0.listFiles(in: Self.worktree, path: "")
        }
        await record("readFile", ["files", "cat", "--json", "--", "w1", "src/main.rs"]) {
            _ = await $0.readFile(in: Self.worktree, path: "src/main.rs")
        }
        // An extra read-only folder's two reads (ov-232): by name, never by id.
        await record("listFiles in a folder", ["files", "folder-ls", "--json", "--", "logs", "nginx"]) {
            _ = await $0.listFiles(inFolder: "logs", path: "nginx")
        }
        await record("readFile in a folder", ["files", "folder-cat", "--json", "--", "logs", "nginx/a.log"]) {
            _ = await $0.readFile(inFolder: "logs", path: "nginx/a.log")
        }
        // A name shaped like a flag is still a name.
        await record("readFile named like a flag", ["files", "folder-cat", "--json", "--", "logs", "--runner=x"]) {
            _ = await $0.readFile(inFolder: "logs", path: "--runner=x")
        }
        await record("assign", ["worktree", "assign", "w1", "--to", Self.billing.id, "--json"]) {
            _ = await $0.assignWorktree(Self.worktree, to: Self.billing)
        }
        // The "No orchestrator" row and the workspace header's menu.
        await record(
            "startOrchestrator",
            ["workspace", "start-orchestrator", Self.billing.id, "--harness", "cursor", "--json"]
        ) {
            _ = await $0.startOrchestrator(Self.billing, harness: .cursor, replace: false)
        }
        await record(
            "replaceOrchestrator",
            ["workspace", "start-orchestrator", Self.billing.id, "--harness", "codex", "--replace", "--json"]
        ) {
            _ = await $0.startOrchestrator(Self.billing, harness: .codex, replace: true)
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

    /// The one task write the board makes, against a stub, with the line it
    /// sent. `exercise()` doesn't carry it: it's a task call, and
    /// `everyLineTheseCallsSendParsesInTheCLI` adds it by name.
    private func taskWrites() async -> (answer: [[String]], none: Void) {
        let answer = Recorder()
        _ = await client(answer).answerDecision(key: "-9", body: "SQLite", repository: "repo")
        return (answer.calls, ())
    }

    /// Answering a decision appends an ANSWER note to the record (spec §2.5):
    /// the option's text, as the person, and nothing moved. The record is
    /// append-only, so an answer is a note, never an edit.
    @Test("Answering a decision sends task note as an answer")
    func answeringADecisionSendsTaskNoteAsAnAnswer() async {
        let sent = await taskWrites()
        #expect(
            sent.answer == [
                ["task", "note", "-9", "--kind", "answer", "--body", "SQLite", "--repo", "repo", "--json"]
            ])
    }

    /// A runner with `needs_you` is read with the one command whose JSON is
    /// the client core's, and what it said becomes this runner's items. A
    /// runner without it isn't asked at all: its blocked agents are derived
    /// from the fleet instead, and its section says to update it.
    @Test("Needs You is read with farcooler needs-you --json")
    func needsYouIsReadWithFarcoolerNeedsYouJSON() async {
        let current = Recorder()
        current.capabilities = ["workspaces", "terminals", "needs_you"]
        let reader = client(current)
        await reader.refreshNeedsYou()
        #expect(current.calls.contains(["needs-you", "--json"]), "\(current.calls)")
        #expect(reader.needsYouList?.map(\.itemID) == ["decision:t-9"])
        #expect(reader.needsYouList?.first?.task?.key == "bil-9")
        #expect(!reader.needsYouFromOlderRunner)

        let older = Recorder()
        older.capabilities = ["workspaces", "terminals", "workstreams"]
        let old = client(older)
        await old.refreshNeedsYou()
        #expect(!older.calls.contains { $0.first == "needs-you" }, "\(older.calls)")
        #expect(old.needsYouFromOlderRunner)
    }

    /// A needs-you read that fails isn't a read: the board's pill counts the
    /// Needs Decision column, as it does before the first read, rather than
    /// the empty list's 0. Once a read succeeds, a later failure keeps that
    /// list and its count (M4 of the night review, ov-76).
    @Test("A failed Needs You read leaves the board's pill on the column's count")
    func aFailedNeedsYouReadLeavesThePillOnTheColumn() async {
        let recorder = Recorder()
        recorder.capabilities = ["workspaces", "terminals", "needs_you"]
        recorder.needsYouFails = true
        let reader = client(recorder)
        await reader.refreshNeedsYou()
        #expect(reader.needsYouKnown, "the window still settles on a runner that can't say")
        #expect(reader.boardWaiting(columnCount: 2, decisions: 0) == 2)

        recorder.needsYouFails = false
        await reader.refreshNeedsYou()
        #expect(reader.boardWaiting(columnCount: 2, decisions: 1) == 1)
        recorder.needsYouFails = true
        await reader.refreshNeedsYou()
        #expect(reader.boardWaiting(columnCount: 2, decisions: 1) == 1, "the last good list stands")
    }

    /// A read that succeeds with nothing in it, after one that failed,
    /// changes no list, so it's the flag alone that has to tell the window
    /// the pill can now say 0: published, or the pill kept the column's
    /// count until something else redrew it.
    @Test("An empty read after a failed one redraws the board's pill")
    func anEmptyReadAfterAFailedOneRedrawsThePill() async {
        let recorder = Recorder()
        recorder.capabilities = ["workspaces", "terminals", "needs_you"]
        recorder.needsYouFails = true
        let reader = client(recorder)
        await reader.refreshNeedsYou()
        #expect(reader.boardWaiting(columnCount: 2, decisions: 0) == 2)

        final class Count { var changes = 0 }
        let count = Count()
        let watching = reader.objectWillChange.sink { _ in count.changes += 1 }
        defer { watching.cancel() }
        recorder.needsYouFails = false
        recorder.needsYouEmpty = true
        await reader.refreshNeedsYou()
        #expect(reader.boardWaiting(columnCount: 2, decisions: 0) == 0)
        #expect(count.changes > 0, "nothing told the window to redraw the pill")
    }

    /// A reconnection clears the runner's build until `status` answers
    /// again, and a runner whose build isn't known yet used to be read as an
    /// older one: every decision and ask swapped for the derived list for a
    /// round trip. The last list stands through the gap, and through a
    /// `status` read that fails.
    @Test("A reconnection keeps the last Needs You list until the build is read")
    func aReconnectionKeepsTheLastNeedsYouList() async {
        let recorder = Recorder()
        recorder.capabilities = ["workspaces", "terminals", "needs_you"]
        let reader = client(recorder)
        await reader.refreshNeedsYou()
        #expect(reader.needsYouList?.map(\.itemID) == ["decision:t-9"])
        recorder.capabilities = nil
        await reader.refresh()
        #expect(reader.daemonBuild == nil, "the reconnection didn't clear the build")
        #expect(reader.needsYouList?.map(\.itemID) == ["decision:t-9"])
        #expect(!reader.needsYouFromOlderRunner)
        reader.stopEvents()
    }

    /// A stream that fell behind re-reads Needs You with everything else it
    /// feeds: a dropped `needs_you` line left the count stale until the
    /// next change.
    @Test("Missed events re-read Needs You")
    func missedEventsReReadNeedsYou() async {
        let recorder = Recorder()
        recorder.capabilities = ["workspaces", "terminals", "needs_you"]
        let reader = client(recorder)
        await reader.refresh()
        await reader.refreshNeedsYou()
        recorder.calls = []
        await reader.eventsMissed()
        #expect(recorder.calls.contains(["needs-you", "--json"]), "\(recorder.calls)")
        reader.stopEvents()
    }

    /// An ask's Allow and Deny send `terminal agent-answer` with the ask's own
    /// ids, exactly as the runner sent them. A refusal is told apart by its
    /// `what:` word: someone else answered, or the agent didn't take it.
    @Test("An ask is answered with terminal agent-answer")
    func anAskIsAnsweredWithTerminalAgentAnswer() async {
        let recorder = Recorder()
        let said = await client(recorder).answerAsk(
            terminal: "t-4", request: "hook-ask-29", option: "allow")
        #expect(said == nil)
        #expect(recorder.calls == [["terminal", "agent-answer", "t-4", "hook-ask-29", "allow", "--json"]])

        func refused(_ stderr: String) async -> DaemonClient.AskRefusal? {
            let recorder = Recorder()
            recorder.answerRefusal = stderr
            return await client(recorder).answerAsk(terminal: "t-4", request: "hook-ask-29", option: "deny")
        }
        #expect(await refused("error: gone\ncode: resource-conflict\nwhat: not_held") == .notHeld)
        #expect(await refused("error: no\ncode: resource-conflict\nwhat: not_delivered") == .notDelivered)
        #expect(await refused("error: no\ncode: resource-conflict") == .failed)
        #expect(DaemonClient.AskRefusal.notHeld.sentence(agent: "claude") == "Someone already answered this.")
        #expect(DaemonClient.AskRefusal.notDelivered.sentence(agent: "claude") == "Couldn’t reach claude. Try again.")
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
        let context = "Couldn’t move “fix it” to Billing."
        #expect(gone.said == "\(context) \(RunnerRefusal.notFound.sentence)")
        #expect(gone.calls.contains { $0.prefix(2) == ["worktree", "list"] }, "no re-read: \(gone.calls)")
        for word in ["capability-unsupported", "scope-denied", "resource-conflict", "invalid-argument"] {
            let shared = RunnerRefusal(rawValue: word)!
            #expect(await refusal("error: no\ncode: \(word)").said == "\(context) \(shared.sentence)", "\(word)")
        }
        #expect(
            await refusal("error: no workspace matching \"0198f2c0\"").said
                == "Couldn’t move “fix it” to Billing. Check that the runner is reachable, then try again.")
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

        // New Workspace….
        let making = Recorder()
        _ = await self.client(making).createWorkspace(repository: "repo", name: "Billing", prefix: "bil")
        #expect(making.calls.first == ["workspace", "create", "--repo", "repo", "--name", "Billing", "--prefix", "bil", "--json"])
        _ = await self.client(making).createWorkspace(repository: "repo", name: "Ops", prefix: "")
        lines += making.calls.filter { $0.first == "workspace" }

        // Use as Orchestrator and Stop Being Orchestrator (ov-63).
        lines += ["orchestrator", "agent", "shell"].map {
            DaemonClient.setRoleArguments(terminal: "5a7573bd", role: $0)
        }

        // Needs You's read, and an ask's answer.
        let needs = Recorder()
        needs.capabilities = ["needs_you"]
        await self.client(needs).refreshNeedsYou()
        lines += needs.calls.filter { $0.first == "needs-you" }
        let answering = Recorder()
        _ = await self.client(answering).answerAsk(terminal: "t-4", request: "hook-ask-29", option: "allow")
        lines += answering.calls

        // The board's one task write, an answer.
        let writes = await taskWrites()
        #expect(!writes.answer.isEmpty)
        lines += writes.answer

        var seen = Set<[String]>()
        for line in lines where seen.insert(line).inserted {
            let (status, stderr) = Self.parse(line, with: cli)
            #expect(status == 0, "farcooler \(line.joined(separator: " ")): \(stderr)")
        }
    }

    /// New Workspace…'s line gets past the CLI's own checks, which `--help`
    /// never reaches: clap answers `--help` before `workspace create`
    /// refuses a missing `--prefix`, which is how a sheet whose prefix was
    /// "optional" always failed. So this runs the real create path against a
    /// runner that can't be reached (`--host` at a name that doesn't
    /// resolve): a line the CLI accepts fails only at connecting. The
    /// prefix-less line is the control, refused before that.
    @Test("New Workspace's line passes the CLI's own checks")
    func newWorkspacesLinePassesTheCLIsOwnChecks() throws {
        let cli = try #require(Self.cli, "No farcooler CLI to run. Build it with cargo build --bin farcooler.")
        let prefix = WorkspacePrefix.derive(name: "Billing", taken: ["fc"])
        let line = DaemonClient.createWorkspaceArguments(repository: "0198f2c0", name: "Billing", prefix: prefix)
        let (status, stderr) = Self.run(["--host", "fc-nowhere.invalid"] + line, with: cli)
        #expect(status != 0)
        #expect(!stderr.contains("--prefix") && !stderr.contains("--name"), "refused by its own checks: \(stderr)")

        let bare = ["workspace", "create", "--repo", "0198f2c0", "--name", "Billing", "--json"]
        let (_, refused) = Self.run(["--host", "fc-nowhere.invalid"] + bare, with: cli)
        #expect(refused.contains("task prefix"), "the control wasn't refused: \(refused)")
        #expect(
            DaemonClient.saidByCLI(refused)?.hasPrefix("Give the workspace a task prefix") == true,
            "\(DaemonClient.saidByCLI(refused) ?? "nil")")
    }

    /// Run `farcooler <line>` with a home of its own, and at most 30 s.
    private static func run(_ line: [String], with cli: String) -> (Int32, String) {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: cli)
        process.arguments = line
        var environment = ProcessInfo.processInfo.environment
        environment["FARCOOLER_HOME"] = "/tmp/fc-t/ui-2/h-\(UUID().uuidString.prefix(8))"
        process.environment = environment
        let err = Pipe()
        process.standardOutput = FileHandle.nullDevice
        process.standardError = err
        process.standardInput = FileHandle.nullDevice
        do { try process.run() } catch { return (-1, "\(error)") }
        let deadline = Date().addingTimeInterval(30)
        while process.isRunning, Date() < deadline { Thread.sleep(forTimeInterval: 0.05) }
        if process.isRunning { process.terminate() }
        let stderr = err.fileHandleForReading.readDataToEndOfFile()
        process.waitUntilExit()
        return (process.terminationStatus, String(decoding: stderr, as: UTF8.self))
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
        // Before a `--`: after one `--help` would be one more name.
        var arguments = line
        arguments.insert("--help", at: line.firstIndex(of: "--") ?? line.endIndex)
        process.arguments = arguments
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
