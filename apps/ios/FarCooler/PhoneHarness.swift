#if DEBUG
import SwiftUI

// The phone's stack over a canned runner, for the UI suite (`-phone-harness`).
//
// Everything on screen is the shipping code: `PhoneRoot`, Needs You, the
// workspace, task and worktree screens, reading a `FleetStore` stood on one
// `Connection` nobody dialed. What is canned is the runner: its fleet, and
// the answers to the calls those screens make, through
// `Connection.standInCalls`. The answers CHECK what they were sent, so a
// screen that sends the wrong task or the wrong option is told no, and the
// test watching it fails, rather than a stub agreeing with anything.
//
// No real agent runs, and nothing is dialed. Launch arguments:
//
//   -phone-harness           the fixture below
//   -phone-empty-inbox       nothing needs you
//   -phone-last-billing      Billing was the last workspace open
//   -phone-list-fails        the runner refuses `needs_you`, so its list is
//                            never read
//   -phone-answer-taken      the runner refuses an ask's answer as not_held
//   -phone-read-scope        this phone holds a Read grant: the runner sends
//                            items without their answers, as the daemon does
//   -deep-link <terminal>    as though a notification for it was tapped
//   -push-task <key>         as though a decision push for that task was
//                            tapped
//   -phone-keep-stack        reopen the stack the last launch kept, rather
//                            than forgetting it
//   -phone-saved-gone        the last launch kept a stack whose task is gone
//   -phone-billing-led       Billing has its orchestrator from the start
//   -phone-onboarding        no runners: the onboarding screen, as a fresh install
//   -phone-no-repositories   the runner lists no repository, workspace or worktree
//   -phone-billing-blank     Billing's board has no task (Main's already has none)
//   -phone-claude-missing    the runner found codex and cursor-agent, not claude
//   -phone-none-installed    the runner found none of the three agents
//   -phone-draft-refused     the runner won't paste Ask the Orchestrator's draft, so it's copied
//   -phone-start-127         a started orchestrator's pane ends at once, exit 127
//   -phone-webhooks-hidden   fc-3-webhooks is put away, in Billing's Hidden
//   -phone-start-states     Billing's board also holds bil-11 (a subagent running,
//                            in the orchestrator's pane), bil-12 (second in the
//                            build line) and bil-13 (waiting on bil-9); implies
//                            -phone-billing-led
//   -phone-usage-old         the runner is older than spend: no agent_usage
//   -phone-usage-fails       the runner doesn't answer usage.task
//   -phone-task-fails        the runner refuses task.get, so a task has no record
//   -phone-hide-fails        the runner refuses worktree.hide and worktree.unhide
//   -phone-files-old         the runner is older than Files: no worktree_files, no read_only_folders
//   -phone-board-first       Billing opens on its Board segment, for a capture that sends no input
//   -phone-board-reads       the runner keeps read state (`board_reads`): Billing's floor is
//                            25 hours back, so bil-5 (done a day ago) and bil-7 (moved
//                            ten minutes ago) are unread, and `workspace.mark_read` raises it
//
// A Darwin notification from the test stands in for a notification tapped
// while the app is open: `com.farcooler.harness.agent`, the blocked
// agent's; `com.farcooler.harness.decision`, bil-7's.

struct PhoneHarness: View {
    static var isRequested: Bool { CommandLine.arguments.contains("-phone-harness") }
    private static var onboarding: Bool { CommandLine.arguments.contains("-phone-onboarding") }

    @StateObject private var hosts = RunnerStore()
    @StateObject private var fleet: FleetStore
    @State private var runner: HarnessRunner
    @State private var pendingDestination: Destination?
    /// Whether the canned runner has been stood up: its fleet, its list and
    /// its boards. A UI test waits on this (`phone-harness-ready`) before
    /// anything else, rather than on a guess at how long a first launch
    /// after an install takes.
    @State private var ready = false

    init() {
        let connection = Connection()
        let runner = HarnessRunner(connection: connection)
        _runner = State(initialValue: runner)
        _fleet = StateObject(
            wrappedValue: FleetStore.standIn(on: connection, host: HarnessRunner.host))
        _pendingDestination = State(
            initialValue: UserDefaults.standard.string(forKey: "deep-link").map(Self.pane)
                ?? UserDefaults.standard.string(forKey: "push-task").map(Self.task))
        Self.forgetOnce()
        _ = HarnessTaps.listening
    }

    /// What a previous launch left behind, cleared once per process: SwiftUI
    /// builds this view's value more than once, and clearing on each would
    /// forget what the test under way just chose.
    private static let forgotten: Void = {
        if CommandLine.arguments.contains("-phone-last-billing") {
            UserDefaults.standard.set(
                PhoneWorkspace(
                    runner: HarnessRunner.host.id.uuidString, workspace: HarnessRunner.billing
                ).stored,
                forKey: PhoneLaunch.lastWorkspaceKey)
        } else {
            UserDefaults.standard.removeObject(forKey: PhoneLaunch.lastWorkspaceKey)
        }
        if CommandLine.arguments.contains("-phone-saved-gone") {
            let billing = PhoneWorkspace(
                runner: HarnessRunner.host.id.uuidString, workspace: HarnessRunner.billing)
            UserDefaults.standard.set(
                PhoneLaunch.encode([.workspace(billing), .task(billing, task: HarnessRunner.goneTask)]),
                forKey: PhoneLaunch.stackKey)
        } else if !CommandLine.arguments.contains("-phone-keep-stack") {
            UserDefaults.standard.removeObject(forKey: PhoneLaunch.stackKey)
        }
        for key in UserDefaults.standard.dictionaryRepresentation().keys
        where key.hasPrefix("workspace.segment.") || key.hasPrefix("board.collapsed.")
            || key.hasPrefix("board.read.")
        {
            UserDefaults.standard.removeObject(forKey: key)
        }
        if CommandLine.arguments.contains("-phone-board-first") {
            WorkspaceSegment.board.remember(
                for: PhoneWorkspace(runner: HarnessRunner.host.id.uuidString, workspace: HarnessRunner.billing))
        }
    }()

    private static func forgetOnce() { _ = forgotten }

    /// What a tapped agent notification or card names: a pane, on no runner
    /// in particular, as a push from an older runner does.
    private static func pane(_ terminal: String) -> Destination {
        Destination(place: .terminal(terminal))
    }

    /// What a tapped decision push names: a task by its key.
    private static func task(_ key: String) -> Destination {
        Destination(place: .task(workspace: nil, task: .init(key: key)), question: true)
    }

    var body: some View {
        Group {
            if Self.onboarding {
                HostOnboardingView(hosts: hosts)
            } else {
                PhoneRoot(fleet: fleet, hosts: hosts, pendingDestination: $pendingDestination)
            }
        }
            .overlay(alignment: .topLeading) { snapshotProbe }
            .overlay(alignment: .bottomLeading) { sentProbe }
            // A notification tapped while the app is open: an agent's, by
            // its terminal, as `FleetView` hands one over, or bil-7's
            // decision, by its task. Posted by the UI test as a Darwin
            // notification (`HarnessTaps`).
            .onReceive(NotificationCenter.default.publisher(for: HarnessTaps.agent)) { _ in
                pendingDestination = Self.pane(HarnessRunner.agent)
            }
            .onReceive(NotificationCenter.default.publisher(for: HarnessTaps.decision)) { _ in
                pendingDestination = Self.task("bil-7")
            }
            .overlay(alignment: .topTrailing) {
                if ready {
                    Rectangle()
                        .fill(Color.white.opacity(0.001))
                        .frame(width: 1, height: 1)
                        .accessibilityElement()
                        .accessibilityIdentifier("phone-harness-ready")
                }
            }
            .task {
                await runner.stand()
                fleet.republish()
                ready = true
            }
    }
}

extension PhoneHarness {
    /// What the glances would count: `needsYou=<n>` off the snapshot the
    /// widget, the complication and the watch read, or `needsYou=-` for none.
    /// Sampled, since the file isn't observable; a one-point element, as the
    /// shell's probes are.
    private var snapshotProbe: some View {
        TimelineView(.periodic(from: .now, by: 0.5)) { _ in
            Rectangle()
                .fill(Color.white.opacity(0.001))
                .frame(width: 1, height: 1)
                .accessibilityElement()
                .accessibilityIdentifier("snapshot-probe")
                .accessibilityValue(
                    "needsYou=\(SnapshotStore.read()?.needsYou.map { String($0.count) } ?? "-")")
        }
    }

    /// What the screens sent the canned runner, one write per line
    /// (`HarnessRunner.sent`), for a test to check the words and not only
    /// that the runner took them. Sampled, as `snapshotProbe` is.
    private var sentProbe: some View {
        TimelineView(.periodic(from: .now, by: 0.5)) { _ in
            Rectangle()
                .fill(Color.white.opacity(0.001))
                .frame(width: 1, height: 1)
                .accessibilityElement()
                .accessibilityIdentifier("harness-sent")
                .accessibilityValue(runner.sent.joined(separator: "\n"))
        }
    }
}

/// The two Darwin notifications a UI test posts across the process line,
/// as ordinary notifications here.
enum HarnessTaps {
    static let agent = Notification.Name("com.farcooler.harness.agent")
    static let decision = Notification.Name("com.farcooler.harness.decision")

    /// Listen, once per process.
    static let listening: Void = {
        for name in [agent, decision] {
            CFNotificationCenterAddObserver(
                CFNotificationCenterGetDarwinNotifyCenter(), nil,
                { _, _, name, _, _ in
                    guard let raw = name?.rawValue as String? else { return }
                    DispatchQueue.main.async {
                        NotificationCenter.default.post(name: Notification.Name(raw), object: nil)
                    }
                },
                name.rawValue as CFString, nil, .deliverImmediately)
        }
    }()
}

/// The canned runner: what it has, and how it answers.
@MainActor
final class HarnessRunner {
    /// Fixed, so a workspace remembered by one launch is the same one the
    /// next launch finds.
    static let host = Runner(
        id: UUID(uuidString: "0198F2C0-0000-7000-8000-00000000FACE")!,
        label: "studio", address: "harness.invalid", user: "harness")

    static let repository = "0198f2c0-0000-7000-8000-00000000a001"
    static let main = "0198f2c0-0000-7000-8000-00000000b001"
    static let billing = "0198f2c0-0000-7000-8000-00000000b002"
    static let checkout = "0198f2c0-0000-7000-8000-00000000c001"
    static let webhooks = "0198f2c0-0000-7000-8000-00000000c002"
    static let scratch = "0198f2c0-0000-7000-8000-00000000c003"
    static let mainOrchestrator = "0198f2c0-0000-7000-8000-00000000d001"
    static let agent = "0198f2c0-0000-7000-8000-00000000d002"
    static let shell = "0198f2c0-0000-7000-8000-00000000d003"
    static let billingOrchestrator = "0198f2c0-0000-7000-8000-00000000d004"
    static let decisionTask = "0198f2c0-0000-7000-8000-00000000e007"
    /// `-phone-start-states`: the cards that say who is on them and when they start.
    static var startStates: Bool { CommandLine.arguments.contains("-phone-start-states") }
    static let subagentTask = "0198f2c0-0000-7000-8000-00000000e011"
    static let queuedTask = "0198f2c0-0000-7000-8000-00000000e012"
    static let blockedTask = "0198f2c0-0000-7000-8000-00000000e013"
    static let agentTask = "0198f2c0-0000-7000-8000-00000000e009"
    static let doneTask = "0198f2c0-0000-7000-8000-00000000e005"
    /// A task no board has: what a kept stack names once it's deleted.
    static let goneTask = "0198f2c0-0000-7000-8000-00000000e404"

    private let connection: Connection
    /// Whether Billing has an orchestrator yet. It starts without one,
    /// unless `-phone-billing-led`.
    private var billingLed =
        CommandLine.arguments.contains("-phone-billing-led") || HarnessRunner.startStates
    /// Whether fc-3-webhooks is put away, as the runner keeps it: starts so
    /// under `-phone-webhooks-hidden`, and `worktree.unhide` clears it.
    private var webhooksHidden = CommandLine.arguments.contains("-phone-webhooks-hidden")
    /// Whether a started orchestrator's pane has ended at once, exit 127
    /// (`-phone-start-127`).
    private var billingDead = false
    /// The items still waiting, by id.
    private var waiting: [String]
    /// Every write the screens made, as `harness-sent` shows it:
    /// `task.note <task> <body>`, `start <workspace> <harness> replace=<b>`.
    private(set) var sent: [String] = []
    /// Billing's read state, as a runner that keeps it does (`-phone-board-reads`):
    /// a floor and each opened task's mark, only ever raised.
    private let standUpAt = Int64(Date().timeIntervalSince1970 * 1000)
    private lazy var readFloor = standUpAt - 25 * 3_600_000
    private var readMarks: [String: Int64] = [:]
    private static var keepsReads: Bool { CommandLine.arguments.contains("-phone-board-reads") }

    init(connection: Connection) {
        self.connection = connection
        waiting =
            CommandLine.arguments.contains("-phone-empty-inbox")
            ? [] : ["ask:hook-ask-1", "decision:\(Self.decisionTask)"]
    }

    func stand() async {
        connection.standInCalls = { [weak self] method, args in
            guard let self else { throw ClientCore.CoreError.notStarted }
            return try await self.answer(method, args)
        }
        connection.standIn(
            on: fleet(),
            repositories: CommandLine.arguments.contains("-phone-no-repositories")
                ? []
                : [
                    Repository(
                        id: Self.repository, short: "a001", displayName: "overnight", remote: "")
                ],
            build: DaemonBuild(
                version: "harness", matches: true, platform: "harness",
                capabilities: Set(
                    ["tasks", "needs_you", "workstreams", "terminal_task"]
                        + (Self.keepsReads ? ["board_reads"] : [])
                        + (CommandLine.arguments.contains("-phone-files-old") ? [] : ["worktree_files", "read_only_folders"])
                        + (CommandLine.arguments.contains("-phone-usage-old") ? [] : ["agent_usage"])
                        + (CommandLine.arguments.contains("-phone-queue-old") ? [] : ["agent_queue"])),
                grantedScope: Self.readOnly ? "read" : "control",
                agentsFound: Self.agentsFound,
                readOnlyFolders: CommandLine.arguments.contains("-phone-files-old") ? nil : ["logs"]))
        // What a poll does with a fleet: the runner's projection for the
        // glances, which they need before they'll take a Needs You list.
        FleetSnapshotWriter.write(
            fleet: fleet(), inbox: nil, machine: Self.host.label,
            runner: Self.host.id.uuidString)
        await connection.loadNeedsYou()
        await connection.loadBoards()
    }

    // MARK: - What it has

    /// What the runner says it found: all three unless a flag takes some away.
    private static var agentsFound: [String] {
        if CommandLine.arguments.contains("-phone-none-installed") { return [] }
        if CommandLine.arguments.contains("-phone-claude-missing") { return ["codex", "cursor-agent"] }
        return ["claude", "codex", "cursor-agent"]
    }

    private func fleet() -> Fleet {
        if CommandLine.arguments.contains("-phone-no-repositories") {
            return Fleet(runtimeHealthy: true, livePanes: 0, worktrees: [], workspaces: [])
        }
        let now = Date().timeIntervalSince1970 * 1000
        var checkoutTerminals = [
            Terminal(
                id: Self.mainOrchestrator, short: "d001", title: "claude", preset: "claude",
                state: "running", activity: "working", activitySince: now - 60_000,
                epoch: 1, paneMode: "terminal", chatCapable: false, workspace: Self.main,
                role: "orchestrator")
        ]
        if billingLed {
            checkoutTerminals.append(
                Terminal(
                    id: Self.billingOrchestrator, short: "d004", title: "claude",
                    preset: "claude", state: billingDead ? "exited" : "running",
                    exitCode: billingDead ? 127 : nil, activity: "idle", epoch: 1,
                    paneMode: "terminal", chatCapable: false, workspace: Self.billing,
                    role: "orchestrator"))
        }
        return Fleet(
            runtimeHealthy: true, livePanes: 3,
            worktrees: [
                Worktree(
                    id: Self.checkout, short: "c001", repository: Self.repository,
                    task: "overnight", branch: "main", state: "ready",
                    terminals: checkoutTerminals, isMainCheckout: true, workspace: Self.main,
                    openTasks: []),
                Worktree(
                    id: Self.webhooks, short: "c002", repository: Self.repository,
                    task: "fc-3-webhooks", branch: "feat/webhooks",
                    state: webhooksHidden ? "hidden" : "ready",
                    terminals: [
                        Terminal(
                            id: Self.agent, short: "d002", title: "claude", preset: "claude",
                            state: "running", activity: "blocked", activitySince: now - 120_000,
                            blockedQuestion: "Allow touch x", epoch: 1, paneMode: "terminal",
                            chatCapable: false, taskId: Self.agentTask, workspace: Self.billing,
                            role: "agent"),
                        Terminal(
                            id: Self.shell, short: "d003", title: "Terminal 1", preset: "shell",
                            state: "running", epoch: 1, paneMode: "terminal",
                            chatCapable: false, workspace: Self.billing, role: "shell"),
                    ],
                    workspace: Self.billing,
                    openTasks: [
                        NeedsYouTask(
                            id: Self.agentTask, key: "bil-9", title: "Invoice PDF export",
                            status: "in_progress")
                    ]),
                Worktree(
                    id: Self.scratch, short: "c003", repository: Self.repository,
                    task: "scratch", branch: "scratch", state: "ready", terminals: [],
                    openTasks: []),
            ],
            workspaces: [
                WorkspaceSummary(
                    id: Self.main, name: "Main", taskPrefix: "ove", isMain: true, ordinal: 0,
                    repository: Self.repository, orchestrator: Self.mainOrchestrator),
                WorkspaceSummary(
                    id: Self.billing, name: "Billing", taskPrefix: "bil", isMain: false,
                    ordinal: 1, repository: Self.repository,
                    orchestrator: billingLed ? Self.billingOrchestrator : nil),
            ])
    }

    // MARK: - How it answers

    private struct Refused: Error {}

    private func answer(_ method: String, _ args: [String: Any]) async throws -> Data {
        switch method {
        case "needs_you":
            if CommandLine.arguments.contains("-phone-list-fails") {
                throw ClientCore.CoreError.rejected("unavailable", word: "unavailable")
            }
            return try json(["items": items()])
        case "task.list":
            let workspace = args["workspace"] as? String
            var list: [String: Any] = ["tasks": tasks(workspace)]
            if Self.keepsReads, workspace == Self.billing { list["reads"] = readState() }
            return try json(list)
        case "workspace.mark_read" where Self.keepsReads:
            // Raises the floor and the marks, as the runner does, and refuses a
            // write that isn't Billing's.
            guard args["workspace"] as? String == Self.billing else {
                throw ClientCore.CoreError.rejected("bad workspace", word: "invalid-argument")
            }
            if let floor = args["floor_ms"] as? Int64 { readFloor = max(readFloor, floor) }
            let marks = args["opened"] as? [[String: Any]] ?? []
            for mark in marks {
                guard let task = mark["task_id"] as? String, let at = mark["opened_ms"] as? Int64 else {
                    throw ClientCore.CoreError.rejected("bad mark", word: "invalid-argument")
                }
                readMarks[task] = max(readMarks[task] ?? 0, at)
            }
            let ids = marks.compactMap { $0["task_id"] as? String }.sorted().joined(separator: ",")
            sent.append("workspace.mark_read floor=\(args["floor_ms"] as? Int64 ?? 0) tasks=\(ids)")
            return try json(readState())
        case "task.get" where CommandLine.arguments.contains("-phone-task-fails"):
            throw ClientCore.CoreError.rejected("unavailable", word: "unavailable")
        case "task.get":
            guard args["task"] as? String == Self.decisionTask else { return try json(["notes": []]) }
            return try json([
                "notes": waiting.contains("decision:\(Self.decisionTask)")
                    ? [question] : [question, answered]
            ])
        case "usage.task" where CommandLine.arguments.contains("-phone-usage-fails"):
            throw ClientCore.CoreError.rejected("unavailable", word: "unavailable")
        case "usage.task":
            // bil-7 has had two agents on it, one of whose claude turns stated
            // only part of its usage; every other task none yet.
            guard args["task"] as? String == Self.decisionTask else {
                let nothing: [String: Any] = [
                    "task": args["task"] as? String ?? "", "price_table": "2026-09-25",
                    "totals": [String: Any](), "by_harness_model": [Any](),
                ]
                return try json(nothing)
            }
            let claude: [String: Any] = [
                "turns": 9, "turns_partial": 1, "active_ms": 7_800_000, "input_tokens": 41_000, "output_tokens": 18_400,
                "cache_read_tokens": 1_020_000, "cache_write_tokens": 62_000, "cost_reported_micros": 2_870_000,
            ]
            let codex: [String: Any] = [
                "turns": 3, "active_ms": 2_400_000, "input_tokens": 210_000, "output_tokens": 9_000,
                "cache_read_tokens": 64_000, "unpriced_tokens": 283_000,
            ]
            let spent: [String: Any] = [
                "task": Self.decisionTask, "price_table": "2026-09-25",
                "totals": [
                    "turns": 12, "turns_partial": 1, "active_ms": 10_200_000, "input_tokens": 251_000, "output_tokens": 27_400,
                    "cache_read_tokens": 1_084_000, "cache_write_tokens": 62_000, "cost_reported_micros": 2_870_000,
                    "unpriced_tokens": 283_000,
                ],
                "by_harness_model": [
                    ["harness": "claude", "model": "claude-opus-5", "totals": claude] as [String: Any],
                    ["harness": "codex", "model": "gpt-5.5", "totals": codex] as [String: Any],
                ],
            ]
            return try json(spent)
        case "task.note":
            // An answer to bil-7, written as an answer, with words in it.
            guard args["task"] as? String == Self.decisionTask,
                args["kind"] as? String == "answer",
                let body = args["body"] as? String, !body.isEmpty
            else { throw ClientCore.CoreError.rejected("bad answer", word: "invalid-argument") }
            waiting.removeAll { $0 == "decision:\(Self.decisionTask)" }
            sent.append("task.note bil-7 \(body)")
            return try json([:])
        case "terminal.agent_answer" where CommandLine.arguments.contains("-phone-answer-taken"):
            throw ClientCore.CoreError.rejected(
                "Someone already answered this.", word: "resource-conflict", what: "not_held")
        case "terminal.agent_answer":
            guard args["terminal"] as? String == Self.agent,
                args["requestId"] as? String == "hook-ask-1",
                ["allow", "deny"].contains(args["optionId"] as? String ?? "")
            else { throw ClientCore.CoreError.rejected("bad answer", word: "invalid-argument") }
            waiting.removeAll { $0 == "ask:hook-ask-1" }
            return try json([:])
        case "workspace.start_orchestrator":
            guard args["workspace"] as? String == Self.billing,
                ["claude", "codex", "cursor"].contains(args["harness"] as? String ?? "")
            else { throw ClientCore.CoreError.rejected("bad start", word: "invalid-argument") }
            sent.append(
                "start billing \(args["harness"] as? String ?? "") replace=\(args["replace"] as? Bool ?? true)")
            // Lands a moment later, as a real one does, so "Starting
            // Orchestrator…" is on screen long enough to be read.
            Task { @MainActor in
                try? await Task.sleep(for: .seconds(6))
                billingLed = true
                billingDead = CommandLine.arguments.contains("-phone-start-127")
                connection.standIn(on: fleet())
            }
            return try json(["id": Self.billingOrchestrator])
        case "terminal.draft_prompt":
            // Ask the Orchestrator's paste into a terminal orchestrator (ov-241).
            // `-phone-draft-refused`: the runner can't prove the pane safe, so the
            // phone copies the reference instead.
            guard args["terminal"] as? String == Self.billingOrchestrator,
                let text = args["text"] as? String, text.hasPrefix("About bil-")
            else { throw ClientCore.CoreError.rejected("bad draft", word: "invalid-argument") }
            if CommandLine.arguments.contains("-phone-draft-refused") {
                throw ClientCore.CoreError.rejected("not safe to paste", word: "agent-not-connected")
            }
            sent.append("draft billing \(text)")
            return try json([:])
        case "worktree.list_dir", "worktree.read_file":
            return try files(method, args)
        case "worktree.hide", "worktree.unhide":
            if CommandLine.arguments.contains("-phone-hide-fails") {
                throw ClientCore.CoreError.rejected("unavailable", word: "unavailable")
            }
            guard args["worktree"] as? String == Self.webhooks else {
                throw ClientCore.CoreError.rejected("bad worktree", word: "invalid-argument")
            }
            webhooksHidden = method == "worktree.hide"
            sent.append("\(method) fc-3-webhooks")
            connection.standIn(on: fleet())
            return try json([:])
        default:
            throw ClientCore.CoreError.rejected("not in the harness", word: "unimplemented")
        }
    }

    /// A small tree for Files (ov-259): a worktree's, and a folder named logs.
    /// Refuses a call that names both places or neither, as the core does, and
    /// a path that isn't there, as the runner does.
    private func files(_ method: String, _ args: [String: Any]) throws -> Data {
        let worktree = args["worktree"] as? String, folder = args["folder"] as? String
        guard (worktree != nil) != (folder != nil), let path = args["path"] as? String else {
            throw ClientCore.CoreError.rejected("bad files call", word: "invalid-argument")
        }
        func entry(_ name: String, _ kind: String, _ size: Int = 0, to target: String = "") -> [String: Any] {
            ["name": name, "kind": kind, "size": size, "linkTarget": target]
        }
        func file(_ state: String, _ size: Int, _ text: String = "", to target: String = "") -> [String: Any] {
            ["path": path, "state": state, "size": size, "text": text, "linkTarget": target]
        }
        let trees: [String: [String: [[String: Any]]]] = [
            "worktree": [
                "": [
                    entry("src", "directory"), entry("README.md", "file", 1_200), entry("big.log", "file", 3_100_000),
                    entry("logo.png", "file", 12_000_000), entry("latest", "link", to: "src/main.rs"),
                ],
                "src": [entry("main.rs", "file", 40)],
            ],
            "logs": ["": [entry("today.log", "file", 22)]],
        ]
        let place = worktree != nil ? "worktree" : "logs"
        if folder != nil, folder != "logs" { throw ClientCore.CoreError.rejected("no folder", word: "not-found") }
        if method == "worktree.list_dir" {
            guard let entries = trees[place]?[path] else {
                throw ClientCore.CoreError.rejected("not found", word: "not-found")
            }
            return try json(["path": path, "truncated": false, "entries": entries])
        }
        switch (place, path) {
        case ("worktree", "src/main.rs"):
            return try json(file("text", 40, "fn main() {\n    println!(\"hello\");\n}\n"))
        case ("worktree", "README.md"): return try json(file("text", 1_200, "# Billing\r\nInvoices, in PDF.\r\n"))
        case ("worktree", "big.log"): return try json(file("too_large", 3_100_000))
        case ("worktree", "logo.png"): return try json(file("binary", 12_000_000))
        case ("worktree", "latest"): return try json(file("link", 0, to: "src/main.rs"))
        case ("logs", "today.log"): return try json(file("text", 22, "12:00 started\n12:01 ready\n"))
        default: throw ClientCore.CoreError.rejected("not found", word: "not-found")
        }
    }

    /// `-phone-read-scope`.
    static var readOnly: Bool { CommandLine.arguments.contains("-phone-read-scope") }

    private func json(_ object: Any) throws -> Data {
        try JSONSerialization.data(withJSONObject: object)
    }

    private var question: [String: Any] {
        [
            "id": "note-q", "kind": "question", "actor": "manager",
            "at": Int64(Date().timeIntervalSince1970 * 1000) - 600_000,
            "body": "Which PDF library?",
            "extra": ["options": ["pdfkit", "wkhtmltopdf"]],
        ]
    }

    private var answered: [String: Any] {
        [
            "id": "note-a", "kind": "answer", "actor": "user",
            "at": Int64(Date().timeIntervalSince1970 * 1000), "body": "pdfkit",
        ]
    }

    private func items() -> [[String: Any]] {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        var out: [[String: Any]] = []
        if waiting.contains("ask:hook-ask-1") {
            out.append([
                "id": "ask:hook-ask-1", "kind": "ask", "rank": 120, "since": now - 120_000,
                "workspace_id": Self.billing, "workspace_name": "Billing",
                "repository_id": Self.repository,
                "task": [
                    "id": Self.agentTask, "key": "bil-9", "title": "Invoice PDF export",
                    "status": "in_progress",
                ],
                "terminal": [
                    "id": Self.agent, "worktree_id": Self.webhooks, "label": "claude",
                    "role": "agent", "pane_mode": "terminal", "chat_capable": false,
                ],
                "question": "Allow touch x", "detail": "touch x", "ask_id": "hook-ask-1",
                "actions": [
                    ["id": "deny", "title": "Deny", "destructive": true, "primary": false],
                    ["id": "allow", "title": "Allow", "destructive": false, "primary": true],
                ],
            ])
        }
        if waiting.contains("decision:\(Self.decisionTask)") {
            out.append([
                "id": "decision:\(Self.decisionTask)", "kind": "decision",
                "rank": 200_000_600, "since": now - 600_000,
                "workspace_id": Self.billing, "workspace_name": "Billing",
                "repository_id": Self.repository,
                "task": [
                    "id": Self.decisionTask, "key": "bil-7", "title": "Pick a PDF library",
                    "status": "needs_decision",
                ],
                "question": "Which PDF library?",
                "actions": [
                    ["id": "pdfkit", "title": "pdfkit", "destructive": false, "primary": false],
                    [
                        "id": "wkhtmltopdf", "title": "wkhtmltopdf", "destructive": false,
                        "primary": false,
                    ],
                ],
            ])
        }
        // Below Control, the runner sends an item's id, kind, rank, subject
        // and a fixed question, and none of its answers (spec §2.4).
        guard Self.readOnly else { return out }
        return out.map { item in
            var redacted = item
            for key in ["actions", "detail", "ask_id"] { redacted.removeValue(forKey: key) }
            return redacted
        }
    }

    private func readState() -> [String: Any] {
        [
            "workspace_id": Self.billing, "floor_ms": readFloor,
            "opened": readMarks.filter { $0.value > readFloor }.map { ["task_id": $0.key, "opened_ms": $0.value] },
        ]
    }

    private func tasks(_ workspace: String?) -> [[String: Any]] {
        guard workspace == Self.billing, !CommandLine.arguments.contains("-phone-billing-blank")
        else { return [] }
        // The moment the harness stood up, not the moment of each read: a task
        // that finished a day ago finished then, however often it's read, and a
        // read mark above it has to stay above it.
        let now = standUpAt
        func startStates() -> [[String: Any]] {
            guard Self.startStates else { return [] }
            return [
                [
                    "id": Self.subagentTask, "key": "bil-11", "title": "Receipt emails",
                    "status": "in_progress", "status_since": now - 900_000,
                    "workspace": Self.billing,
                    "workers": [
                        [
                            "id": "0198f2c0-0000-7000-8000-00000000f011", "harness": "claude",
                            "agent_id": "a3fd8fceef581c787", "label": "bil-11 Receipt emails",
                            "model": "opus", "state": "running", "started_at": now - 720_000,
                            "ended_at": NSNull(), "last_activity_at": now - 20_000,
                            "doing": "Running cargo test", "linked_by_description": true,
                            "orchestrator_terminal": Self.billingOrchestrator,
                        ]
                    ],
                ],
                [
                    "id": Self.queuedTask, "key": "bil-12", "title": "Refund flow",
                    "status": "in_progress", "status_since": now - 1_800_000,
                    "workspace": Self.billing,
                    "wait": [
                        "kind": "in_line", "line": "build", "position": 2,
                        "ahead": ["bil-11"], "since": now - 600_000,
                    ],
                ],
                [
                    "id": Self.blockedTask, "key": "bil-13", "title": "Tax rounding",
                    "status": "backlog", "status_since": now - 7_200_000,
                    "workspace": Self.billing, "waiting_on": ["bil-9"],
                ],
            ]
        }
        return startStates() + [
            [
                "id": Self.decisionTask, "key": "bil-7", "title": "Pick a PDF library",
                "status": "needs_decision", "status_since": now - 600_000,
                "intent": "Invoices need a PDF.", "workspace": Self.billing,
            ],
            [
                "id": Self.agentTask, "key": "bil-9", "title": "Invoice PDF export",
                "status": "in_progress", "status_since": now - 3_600_000,
                "worktree_id": Self.webhooks, "workspace": Self.billing,
                "acceptance": [
                    ["id": "a1", "text": "Exports a PDF", "met": true],
                    ["id": "a2", "text": "Emails it", "met": false],
                ],
            ],
            [
                "id": Self.doneTask, "key": "bil-5", "title": "Stripe webhooks",
                "status": "done", "status_since": now - 86_400_000, "workspace": Self.billing,
            ],
        ]
    }
}
#endif
