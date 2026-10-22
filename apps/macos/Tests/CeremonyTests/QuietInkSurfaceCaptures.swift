import AgentKit
import AppKit
import SwiftUI
import TestWindows
import Testing

@testable import Far_Cooler

/// Opt-in (FARCOOLER_CAPTURE_OUT): the Plan view and the board's task card on
/// their paper, light and dark, for judging that the hierarchy still reads once
/// quiet text is `Ink.quiet` (ov-687). The production views over the plan
/// fixture the stage captures use, in a real titled window off every screen.
/// For the before, run it on the commit before the change.
@MainActor
@Suite(.serialized, .enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] != nil))
struct QuietInkSurfaceCaptures {
    static let appearances: [(String, NSAppearance.Name)] = [("light", .aqua), ("dark", .darkAqua)]

    /// `view` on the paper a content card is drawn on, in each appearance.
    /// `view` is built again for each appearance, so a store a pane reads from is not shared by the two windows (a closed window cancels its read).
    static func shoot<V: View>(
        _ name: String, width: CGFloat, height: CGFloat, expecting: String, settles: TimeInterval = 0, onPaper: Bool = true,
        until reached: @MainActor () -> Bool, _ view: @autoclosure () -> V
    ) async throws {
        let out = try #require(ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"])
        try FileManager.default.createDirectory(atPath: out, withIntermediateDirectories: true)
        for (variant, appearance) in appearances {
            let window = NSWindow(
                contentRect: NSRect(x: -9000, y: -9000, width: width, height: height), styleMask: [.titled],
                backing: .buffered, defer: false)
            window.isReleasedWhenClosed = false
            // `onPaper: false` paints the window's own ground (`windowBackgroundColor`, which a bare test window does not follow into dark), for a page the app draws no card for.
            window.contentView = NSHostingView(
                rootView: view().frame(width: width, height: height, alignment: .topLeading)
                    .background(onPaper ? WorkspaceStyle.paper : Color(nsColor: .windowBackgroundColor)))
            window.appearance = NSAppearance(named: appearance)
            TestWindow.show(window)
            // The Changes pane keeps its diff invisible until it has restored
            // a scroll position, with a 700 ms deadline held in private state
            // (`settled`): there is nothing to read, so it is waited out.
            if settles > 0 { try await Task.sleep(for: .seconds(settles)) }
            try await Capture.wait(expecting: expecting, until: reached)
            try await Capture.keep(
                to: URL(fileURLWithPath: out).appendingPathComponent("\(name)-\(variant).png"),
                expecting: expecting, until: reached
            ) { window.contentView?.lookBitmap(scale: 2)?.representation(using: .png, properties: [:]) }
            window.close()
        }
    }

    static func store() async throws -> TaskBoardStore {
        let store = try await PlanStageViewTests.store()
        await store.plan.reload()
        return store
    }

    @Test func planHome() async throws {
        let store = try await Self.store()
        try await Self.shoot(
            "quiet-plan-home", width: 760, height: 1_350, expecting: "the plan read",
            until: { store.plan.planned && !store.plan.plan.lanes.isEmpty },
            PlanHome(board: store, needsYou: PlanNeedsYou(), onOpen: { _ in }))
    }

    @Test func lanePage() async throws {
        let store = try await Self.store()
        let plan = store.plan.plan
        try await Self.shoot(
            "quiet-plan-lane", width: 640, height: 700, expecting: "the plan read",
            until: { store.plan.plan.lanes.count >= 5 },
            PlanLanePage(
                lane: plan.lanes[4], plan: plan, record: nil,
                context: PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })))
    }

    @Test func themePage() async throws {
        let plan = try PlanThemePageTests.model()
        let theme = try #require(plan.shownThemes.first)
        try await Self.shoot(
            "quiet-plan-theme", width: 700, height: 900, expecting: "the theme page", until: { true },
            PlanThemePage(
                theme: theme, plan: plan, record: nil,
                context: PlanPageContext(rows: [:], onTask: { _ in }, onOpen: { _ in })))
    }

    static let record = """
        {"task": {}, "notes": [
          {"id": "n1", "kind": "decision", "actor": "manager", "at": 1760000000000,
           "body": "Use the quiet ink for every secondary line on the paper.", "extra": {"rejected": ["Darken .secondary", "A new gray"]}},
          {"id": "n2", "kind": "finding", "actor": "agent:7b1c", "at": 1759990000000, "body": "The system secondary measures 3.8:1 on the paper."},
          {"id": "n3", "kind": "progress", "actor": "agent:7b1c", "at": 1759980000000, "body": "Plan view done; the board is next."},
          {"id": "n4", "kind": "status_change", "actor": "manager", "at": 1759970000000, "body": "todo to in_progress"},
          {"id": "n5", "kind": "created", "actor": "user", "at": 1759960000000, "body": "Mac: secondary text reads in light appearance"}
        ]}
        """

    static let spend = """
        {"task":"t","price_table":"2026-09-25",
         "totals":{"turns":12,"turns_partial":1,"active_ms":10200000,"input_tokens":251000,"output_tokens":27400,
                   "cache_read_tokens":1084000,"cache_write_tokens":62000,"cost_reported_micros":2870000,
                   "unpriced_tokens":283000},
         "by_harness_model":[
           {"harness":"claude","model":"claude-opus-5","totals":{"turns":9,"active_ms":7800000,
             "input_tokens":41000,"output_tokens":18400,"cache_read_tokens":1020000,"cache_write_tokens":62000,
             "cost_reported_micros":2870000}}]}
        """

    @Test func taskCard() async throws {
        let detail = try TaskDetailModel.decode(Data(Self.record.utf8))
        let usage = try TaskUsage.decode(Data(Self.spend.utf8))
        let row = TaskRow(
            id: "t1", key: "ov-687", title: "Mac: secondary text reads in light appearance", status: .inProgress,
            statusSince: Date(timeIntervalSinceNow: -3 * 86_400),
            intent: "In Light Appearance, .secondary text on the Mac measures under 4:1 on the paper.",
            labels: ["found"],
            acceptance: [
                TaskAcceptanceLine(id: "1", text: "Each surface on our own ground reads at 4.5:1", met: true),
                TaskAcceptanceLine(id: "2", text: "System surfaces keep .secondary, with reasons", met: false),
            ],
            constraints: ["No cargo fmt"], createdAt: Date(timeIntervalSinceNow: -5 * 86_400), updatedAt: Date(timeIntervalSinceNow: -3_600))
        try await Self.shoot(
            "quiet-task-card", width: 640, height: 1_000, expecting: "the card", until: { true },
            TaskCard(row: row, detail: detail, question: nil, canAnswer: true, usage: .loaded(usage), onAnswer: { _ in true })
                .padding(TaskTypography.inset))
    }

    // MARK: The Changes and Files panes (second batch, ov-687)

    static let worktree = Worktree(
        id: "co", short: "co", task: "overnight", branch: "main", repository: "overnight", host: "",
        path: "/tmp/overnight", state: "active", terminals: [])

    /// A Changes pane over a branch of two commits and three files, with a
    /// diff that has context, an addition, a removal and a hunk seam.
    static func changesStore(error: String? = nil) -> ChangesStore {
        // The pane reads the worktree when it appears. Answered here, so no
        // command runs: a read that never answers leaves the fixture as set,
        // and a failure is the problem view's.
        let client = DaemonClient(target: "")
        client.commandRunnerForTesting = { _ in
            if let error { return (nil, error) }
            try? await Task.sleep(for: .seconds(3_600))
            return (nil, nil)
        }
        let store = ChangesStore(client: client, worktree: worktree)
        func file(_ path: String, _ status: ChangedFileStatus, _ plus: Int, _ minus: Int) -> ChangedFile {
            ChangedFile(path: path, status: status, oldPath: nil, insertions: plus, deletions: minus, binary: false)
        }
        func commit(_ sha: String, _ subject: String, _ body: String?) -> ChangeCommit {
            ChangeCommit(
                sha: sha, subject: subject, body: body, author: "Ada", timestamp: Int(Date().timeIntervalSince1970) - 7_200,
                filesChanged: 3, insertions: 19, deletions: 6)
        }
        store.changeSet = ChangeSet(
            branch: "mac-687b", baseRef: "main", baseSource: "merge-base", baseCommit: "", headCommit: "",
            insertions: 19, deletions: 6,
            commits: [
                commit("0a1b2c3d4e5f", "Mac: quiet text in the Changes pane (ov-687)", "Primary at 70% on the paper."),
                commit("1b2c3d4e5f6a", "Mac: measure each ground", nil),
            ],
            files: [
                file("apps/macos/Sources/FarCooler/ChangesPane.swift", .modified, 12, 4),
                file("apps/macos/Sources/FarCooler/DiffView.swift", .renamed, 5, 2),
                file("scripts/visual-tokens-baseline.json", .added, 2, 0),
            ], workingTree: nil)
        store.selectedFile = "apps/macos/Sources/FarCooler/ChangesPane.swift"
        let lines: [DiffComputation.Line] = [
            .init(id: 0, kind: .context, oldNumber: 88, newNumber: 88, text: "                    HStack(spacing: 0) {"),
            .init(id: 1, kind: .removed, oldNumber: 89, newNumber: nil, text: "                        Divider()"),
            .init(id: 2, kind: .added, oldNumber: nil, newNumber: 89, text: "                        fileColumn.separator(.split, edge: .trailing)"),
            .init(id: 3, kind: .context, oldNumber: 90, newNumber: 90, text: "                        VStack(spacing: 0) {"),
            .init(id: 4, kind: .context, oldNumber: 140, newNumber: 140, text: "                            diffNavigator(compact: false)"),
            .init(id: 5, kind: .added, oldNumber: nil, newNumber: 141, text: "                            diffBody"),
        ]
        for f in store.changeSet.files { store.fileDiffs[f.path] = FileDiff(lines: lines) }
        store.error = error
        return store
    }

    @Test func changesPane() async throws {
        try await Self.shoot(
            "quiet2-changes", width: 980, height: 440, expecting: "the Changes pane", settles: 1.2, until: { true },
            ChangesPane(changes: Self.changesStore(), isFocused: true, agents: []))
    }

    @Test func changesProblem() async throws {
        try await Self.shoot(
            "quiet2-changes-problem", width: 980, height: 300, expecting: "the Changes problem", until: { true },
            ChangesPane(changes: Self.changesStore(error: "The command that reads it didn’t finish."), isFocused: true, agents: []))
    }

    static let source = """
        //! A worktree's files, read-only.

        use std::path::Path;

        /// The file at `relative` in the worktree at `root`.
        pub fn read(root: &Path, relative: &str) -> Result<Content, Refusal> {
            let path = plain(relative)?;
            Ok(Content::Text { text: path, size: 0 })
        }

        """

    static func filesModel() -> FilesModel {
        let entry = { (name: String, kind: FileEntry.Kind) in FileEntry(name: name, kind: kind, size: 0, linkTarget: "") }
        return FilesModel(
            worktree: worktree,
            source: FilesModel.Source(
                list: { path in
                    switch path {
                    case "": return .success(FileListing(
                        path: "", entries: [entry("apps", .directory), entry("crates", .directory), entry(".env", .file),
                                            entry("Cargo.toml", .file), entry("README.md", .file)], truncated: false))
                    case "crates": return .success(FileListing(
                        path: "crates", entries: [entry("daemon", .directory), entry("protocol", .directory)], truncated: false))
                    case "crates/daemon": return .success(FileListing(
                        path: "crates/daemon", entries: [entry("src", .directory), entry("Cargo.toml", .file)], truncated: false))
                    case "crates/daemon/src": return .success(FileListing(
                        path: "crates/daemon/src", entries: [entry("worktree_files.rs", .file), entry("rpc.rs", .file)], truncated: false))
                    default: return .failure(.missing)
                    }
                },
                read: { path in .success(FileRead(path: path, state: .text, size: 640, text: source, linkTarget: "")) },
                search: { _ in ["crates/daemon/src/main.rs", "crates/cli/src/main.rs", "apps/macos/Sources/FarCooler/FarCoolerApp.swift"] }))
    }

    @Test func filesOpen() async throws {
        let model = Self.filesModel()
        await model.loadIfNeeded()
        await model.open("crates/daemon/src/worktree_files.rs", line: nil)
        model.finding = true
        model.findQuery = "path"
        model.refind()
        try await Self.shoot(
            "quiet2-files-open", width: 980, height: 360, expecting: "the open file", until: { !model.lines.isEmpty },
            FilesPane(model: model))
    }

    @Test func filesFiltered() async throws {
        let model = Self.filesModel()
        await model.loadIfNeeded()
        model.filter = "main"
        await model.runFilter()
        try await Self.shoot(
            "quiet2-files-filtered", width: 980, height: 260, expecting: "the filtered tree", until: { !model.found.isEmpty },
            FilesPane(model: model))
    }

    @Test func diffView() async throws {
        let diff = Diff(
            path: "crates/daemon/src/worktree_files.rs",
            oldText: "let dir = open(root)?;\nlet name = n;\n",
            newText: "let dir = open_dir(root, dirs)?;\nlet name = n;\nlet held = fstat(&fd)?;\n")
        try await Self.shoot(
            "quiet2-diff-view", width: 640, height: 200, expecting: "the diff", until: { true },
            DiffView(diff: diff).padding(12))
    }

    @Test func diffReadFailure() async throws {
        try await Self.shoot(
            "quiet2-diff-read-failure", width: 520, height: 50, expecting: "the failure row", until: { true },
            DiffReadFailureRow(font: .system(size: 11, design: .monospaced)) {})
    }

    // MARK: Setup, panes and columns (third batch, ov-687)

    @Test func welcome() async throws {
        let setup = WelcomeSetup(
            model: WelcomeModel(
                home: "/Users/ada",
                scan: {
                    [
                        RepositorySuggestion(path: "/Users/ada/Dev/harbor", lastCommit: Date().addingTimeInterval(-86_400)),
                        RepositorySuggestion(path: "/Users/ada/Dev/overnight", lastCommit: Date().addingTimeInterval(-9 * 86_400)),
                    ]
                }),
            step: .ready([.claude, .codex]), agents: HarnessAvailability(agentsFound: ["claude", "codex"]), register: { _ in nil },
            onCheckAgain: {}, onAnotherRunner: {})
        await setup.model.look()
        try await Self.shoot(
            "quiet3-welcome", width: 760, height: 760, expecting: "the Welcome page", until: { true },
            WelcomeView(setup: setup).contentCard())
    }

    @Test func charter() async throws {
        let setup = CharterSetupTests.setup()
        try await Self.shoot(
            "quiet3-charter", width: 900, height: 900, expecting: "the Charter", onPaper: false,
            until: { setup.phase == .ready }, CharterSetupView(setup: setup))
    }

    @Test func repColumn() async throws {
        let people = RunnerPeople(members: [], claudeAccount: "team@example.com", you: "bob")
        try await Self.shoot(
            "quiet3-rep-column", width: 420, height: 420, expecting: "the rep column", until: { true },
            RepPlaceholder(
                state: .stopped, intro: people.repIntro, refusal: RunnerPeople.repNotStarted, canAct: true, onStart: {}))
    }

    @Test func tileHeaders() async throws {
        let ids = ["a", "b", "c"]
        var terminals = ids.map { Terminal(id: $0, short: $0, title: $0, preset: "claude", state: "LOST", epoch: 0) }
        terminals[2].paneMode = "changes"
        let worktree = Worktree(
            id: "co", short: "co", task: "overnight", branch: "main", repository: "overnight", host: "",
            path: "/tmp/overnight", state: "active", terminals: terminals)
        let panes = ids.enumerated().map { index, id in
            PaneRect(
                id: id, short: id, title: nil, left: index * 41, top: 0, columns: 39, rows: 24, focused: index == 0, zoomed: false)
        }
        let group = PaneGroup(id: "@1", name: "", active: true, columns: 120, rows: 24, layout: "a", panes: panes)
        let changes = ChangesStore(client: DaemonClient(target: ""), worktree: worktree)
        try await Self.shoot(
            "quiet3-tile-headers", width: 1_100, height: 140, expecting: "the pane headers", onPaper: false, until: { true },
            TileView(
                groups: [group], showing: group.id, worktree: worktree, changes: changes,
                binary: nil, environment: [:], hostArguments: [], linkGeneration: 0,
                onFocus: { _ in }, onSelectGroup: { _ in }, onDropOnPane: { _, _, _ in },
                onViewport: { _, _, _ in }, onResizeDivider: { _, _, _ in true },
                title: "Main", subtitle: "", setsTitle: false))
    }

    @Test func lfsNotice() async throws {
        let client = DaemonClient(target: "")
        var tracked = Self.worktree
        tracked.lfsPointers = 3
        client.fleet = Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [tracked], branchPrefix: nil)
        try await Self.shoot(
            "quiet3-lfs-notice", width: 640, height: 80, expecting: "the notice", until: { true },
            ChangesLfsNotice(client: client, worktree: tracked.id))
    }
}
