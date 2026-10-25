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
//   -phone-long-options      bil-7's question offers three options, two of them long
//   -push-task <key>         as though a decision push for that task was
//                            tapped
//   -phone-keep-stack        reopen the stack the last launch kept, rather
//                            than forgetting it
//   -phone-keep-places       keep where each workspace was left (ov-442) rather than
//                            forgetting it, to open one again after a relaunch
//   -phone-stack-lags        a placed stack takes its pushes 0.3 s late, as a
//                            loaded simulator's did (ov-337)
//   -phone-saved-gone        the last launch kept a stack whose task is gone
//   -phone-no-orchestrators  Main has no orchestrator either: none anywhere
//   -phone-billing-led       Billing has its orchestrator from the start
//   -phone-onboarding        no runners: the onboarding screen, as a fresh install
//   -phone-many-repos        three more repositories with long names, each with its main checkout
//                            and an idle orchestrator named for its work (ov-676)
//   -phone-no-repositories   the runner lists no repository, workspace or worktree
//   -phone-billing-blank     Billing's board has no task (Main's already has none)
//   -phone-claude-key-warning  Runner Settings' Health shows the owner's shell-exports-a-key warning (ov-651)
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
//   -phone-loose-hidden      Billing also owns spike, a worktree no card reaches, put away:
//                            the tree's Loose Worktrees › Hidden (ov-300)
//   -phone-reopen-worktree   the last launch kept the checkout worktree open to resume, with
//                            Changes chosen in it (ov-233); implies -phone-keep-stack
//   -phone-no-kept-focus     with -phone-reopen-worktree, nothing was chosen in it: the rule decides
//   -pad-compact             on an iPad, the app as a one-third Split View draws it: 375 points
//                            wide at compact width (ov-348, `PadCompactWindow`); the
//                            com.farcooler.harness.pad-compact notice turns it on and off
//   -pad-compact-wide        that window 800 points wide instead: compact by its size class alone
//   -phone-files-old         the runner is older than Files: no worktree_files, no read_only_folders
//   -phone-plan, -phone-plan-fails, -phone-plan-hangs, -phone-plan-down, -phone-plan-timeout N,
//                            -phone-plan-file <path>: the plan layer (ov-274), see
//                            PhonePlanHarness.swift
//   -phone-plan-was-chosen   Plan was chosen on Billing's board before this runner lost board_plan
//   -phone-terminal-key      the shell terminal (d003) opens with a line naming bil-9, a task on
//                            Billing's board, for a long press to land on (ov-215)
//   -phone-terminal-alt      with -phone-terminal-key, that screen is a full-screen program's (the
//                            alternate screen, mouse reporting on), and each terminal.write is in
//                            `sent` as `write <hex>` (ov-609)
//   -phone-shared            the runner is shared (ov-508): it serves `member.list`, two
//                            members besides the owner, one with a long name and reps in
//                            Main and Billing, and this phone is Bob's
//   -phone-rep               a member's Talk (ov-526): shared as above, with Bob's rep in Billing
//                            not started (the phone starts it, taking three seconds), and Billing
//                            led by Alexandra. Add -phone-rep-running or -phone-rep-stopped for a
//                            rep that exists, -phone-rep-lead for Bob leading Billing (he keeps the
//                            orchestrator, so pair it with -phone-billing-led), -phone-rep-refused
//                            for a runner that won't start it, -phone-rep-raced for one that says the
//                            rep is running as it starts, -phone-rep-slow-list and -phone-rep-list-fails
//                            for a member.list that takes eight seconds or never answers,
//                            -phone-rep-owner for the owner's own phone (you is empty), and
//                            -phone-rep-keep-you to keep what the phone remembered of who it is
//                            across launches (cleared otherwise), and -phone-rep-long for long names
//   -phone-native            Main's orchestrator is claude with its conversation served (ov-611):
//                            the runner offers agent rows and compose and answers them as
//                            `-native-agent-harness` does (`NativeHarnessRunner`, with its own
//                            flags), so Main shows the conversation under its Now/Next card, as
//                            the owner's phone does. With -native-agents its four subagents run,
//                            named in the fleet too, for the Agents card (ov-619)
//   -phone-claude            a member signs in to Claude from the runner's settings (ov-584): shared as
//                            above, with `claude_sign_in`, and Bob not signed in. Add -phone-claude-signed-in
//                            or -phone-claude-stopped for the login he already has, -phone-claude-line for his
//                            Claude login line in Needs You (ov-587; implies stopped), -phone-claude-slow for a link
//                            that takes six seconds, -phone-claude-too-often for a runner that refuses
//                            the start, -phone-claude-expired for a code that finds the link gone, -phone-claude-owner for the owner's own phone, and
//                            -phone-host-long for a long runner name; -phone-rep-long makes the email long
//   -phone-own-logins        a shared runner's Member Logins switches (ov-652): Codex's on, the
//                            others off; only the owner's phone (-phone-rep-owner) may turn them,
//                            and each turn is in `sent` as `own-login <harness> <on>`; add
//                            -phone-own-logins-refused for a runner that refuses every turn
//   -phone-member-notes      bil-7's record written by people (ov-707): Alexandra's answer, the
//                            owner's comment and Bob's; with -phone-shared, as a member's phone
//                            (Bob's) or the owner's (-phone-rep-owner) reads it
//   -phone-team              a runner with a team (ov-633): the ask is this phone's own
//                            (Alice Chen-Montgomery's), the decision a teammate's with a
//                            long name, and Main's orchestrator is blocked on the owner's
//                            question, so Needs You splits into Yours and Everyone Else
//   -phone-board-reads       the runner keeps read state (`board_reads`): Billing's floor is
//                            25 hours back, so bil-5 (done a day ago) and bil-7 (moved
//                            ten minutes ago) are unread, and `workspace.mark_read` raises it
//
// A Darwin notification from the test stands in for a notification tapped
// while the app is open: `com.farcooler.harness.agent`, the blocked
// agent's; `com.farcooler.harness.decision`, bil-7's. And
// `com.farcooler.harness.link-lost` stands in for a link gone quiet: the
// fleet held is the last link's, and the runner isn't answering.

struct PhoneHarness: View {
    static var isRequested: Bool { CommandLine.arguments.contains("-phone-harness") }
    /// `-phone-stack-lags`: see `PhoneNavigator.push`.
    static var stackLags: Bool { isRequested && CommandLine.arguments.contains("-phone-stack-lags") }
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
    /// The phone's own Light or Dark as this app sees it, which the probe
    /// carries, so a test of the appearance knows the phone it was given
    /// (`HarnessAppearance`).
    @Environment(\.colorScheme) private var systemScheme

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
        } else if CommandLine.arguments.contains("-phone-reopen-worktree") {
            // The last run: the checkout open to resume, Changes the tab
            // chosen in it, as `FocusMemory` kept it.
            let runner = HarnessRunner.host.id.uuidString
            UserDefaults.standard.set(
                PhoneLaunch.encode([.worktree(runner: runner, worktree: HarnessRunner.checkout, landing: .resume)]),
                forKey: PhoneLaunch.stackKey)
            FocusMemory<PaneFocus>.save(
                CommandLine.arguments.contains("-phone-no-kept-focus") ? [:] : [HarnessRunner.checkout: .changes],
                runner: runner)
        } else if !CommandLine.arguments.contains("-phone-keep-stack") {
            UserDefaults.standard.removeObject(forKey: PhoneLaunch.stackKey)
        }
        if !CommandLine.arguments.contains("-phone-reopen-worktree") {
            FocusMemory<PaneFocus>.save([:], runner: HarnessRunner.host.id.uuidString)
        }
        let keepPlaces = CommandLine.arguments.contains("-phone-keep-places")
        for key in UserDefaults.standard.dictionaryRepresentation().keys
        where key.hasPrefix("workspace.segment.")
            || (!keepPlaces && (key.hasPrefix("workspace.place.") || key.hasPrefix("workspace.canvas.")))
            || key.hasPrefix("board.collapsed.")
            || key.hasPrefix("board.read.") || key.hasPrefix("board.plan.") || key.hasPrefix("tree.filter.")
            || key.hasPrefix("pad.tree.")
        {
            UserDefaults.standard.removeObject(forKey: key)
        }
        // Every draft, photo and row cache the last launch kept, for every pane.
        HarnessDrafts.resetAll()
        if CommandLine.arguments.contains("-phone-plan-was-chosen") {
            // Plan was chosen on Billing's board before this runner lost the layer.
            PlanChoice.set(
                true, host: HarnessRunner.host.id.uuidString, workspace: HarnessRunner.billing)
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
                    .modifier(PadCompactWindow())
                    // The native harness's photo and paste notices, into Main's
                    // orchestrator's composer (`-phone-native`).
                    .modifier(NativePhotoHarness(pane: HarnessRunner.mainOrchestrator))
            }
        }
            .overlay(alignment: .topLeading) { snapshotProbe }
            .overlay(alignment: .bottomLeading) { sentProbe }
            .overlay(alignment: .bottomTrailing) { PasteboardProbe() }
            .overlay(alignment: .center) { HarnessStoresProbe() }
            .environment(\.openURL, OpenURLAction { url in
                runner.opened(url)
                return .handled
            })
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
            // The link going quiet: connected, its fleet the last link's, so
            // nothing about now can be said (ov-619 review).
            .onReceive(NotificationCenter.default.publisher(for: HarnessTaps.linkLost)) { _ in
                runner.loseLink()
            }
            // The plan layer's news (ov-274), delivered where the client core's would be.
            .onReceive(NotificationCenter.default.publisher(for: HarnessTaps.planNews)) { _ in
                Task { await runner.deliver(["event": "plan", "workspace": HarnessRunner.billing]) }
            }
            // A page written (ov-285), as the client core's `pages` line says it.
            .onReceive(NotificationCenter.default.publisher(for: HarnessTaps.pagesNews)) { _ in
                Task {
                    await runner.deliver([
                        "event": "pages", "workspace": HarnessRunner.billing, "slot": "train", "removed": false,
                    ])
                }
            }
            .onReceive(NotificationCenter.default.publisher(for: HarnessTaps.taskNews)) { _ in
                Task {
                    await runner.deliver([
                        "event": "task", "repository": HarnessRunner.repository, "workspace": HarnessRunner.billing,
                    ])
                }
            }
            .overlay(alignment: .topTrailing) {
                if ready {
                    Rectangle()
                        .fill(Color.white.opacity(0.001))  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
                        .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
                        .accessibilityElement()
                        .accessibilityIdentifier("phone-harness-ready")
                        .accessibilityValue(systemScheme == .dark ? "dark" : "light")
                }
            }
            .task {
                HarnessAppearance.apply()
                await runner.stand()
                fleet.republish()
                ready = true
            }
    }
}

/// `-phone-appearance dark` or `light`: the phone's own Light or Dark for
/// this launch, set on the window, which is where a change in Settings
/// reaches the app's views and the system's sheets alike. Not
/// `-AppleInterfaceStyle`, which SwiftUI reads and a system sheet's glass
/// does not; and not only `XCUIDevice.appearance`, which on a fresh iOS
/// 27 simulator (CI's) reported the switch while every app launched after
/// the first in the run kept the first one's appearance. Both harnesses
/// call it (`PhoneHarness`, `NativeAgentHarness`), so the flag means the
/// same under either (ov-732).
enum HarnessAppearance {
    static func apply() {
        let arguments = CommandLine.arguments
        guard let at = arguments.firstIndex(of: "-phone-appearance"), at + 1 < arguments.count else { return }
        let style: UIUserInterfaceStyle = arguments[at + 1] == "dark" ? .dark : .light
        for case let scene as UIWindowScene in UIApplication.shared.connectedScenes {
            for window in scene.windows { window.overrideUserInterfaceStyle = style }
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
                .fill(Color.white.opacity(0.001))  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
                .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
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
                .fill(Color.white.opacity(0.001))  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
                .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
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
    static let planNews = Notification.Name("com.farcooler.harness.plan-news")
    static let linkLost = Notification.Name("com.farcooler.harness.link-lost")
    static let taskNews = Notification.Name("com.farcooler.harness.task-news")
    static let pagesNews = Notification.Name("com.farcooler.harness.pages-news")
    /// `PadCompactWindow` on or off.
    static let padCompact = Notification.Name("com.farcooler.harness.pad-compact")
    /// A small photo, a 10 MB one, and an image pasted, into the conversation
    /// composer (`NativeAgentHarness`, ov-404).
    static let nativePhoto = Notification.Name("com.farcooler.harness.native-photo")
    static let nativeBigPhoto = Notification.Name("com.farcooler.harness.native-big-photo")
    static let nativePaste = Notification.Name("com.farcooler.harness.native-paste")
    /// Lets the older page of `-native-live-older` through (`NativeLiveRows.older`, ov-629).
    static let nativeOlder = Notification.Name("com.farcooler.harness.native-older")
    /// A photo picked into a terminal-mode orchestrator (`OrchestratorImageDoor`, ov-444).
    static let orchestratorPhoto = Notification.Name("com.farcooler.harness.orchestrator-photo")

    /// Listen, once per process.
    static let listening: Void = {
        for name in [agent, decision, linkLost, planNews, taskNews, pagesNews, padCompact, nativePhoto, nativeBigPhoto, nativePaste, orchestratorPhoto, nativeOlder] {
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
        label: CommandLine.arguments.contains("-phone-host-long")
            ? "Elias’s Mac Studio in the Platform Infrastructure Lab" : "studio",
        address: "harness.invalid", user: "harness")

    static let repository = "0198f2c0-0000-7000-8000-00000000a001"
    static let main = "0198f2c0-0000-7000-8000-00000000b001"
    static let billing = "0198f2c0-0000-7000-8000-00000000b002"
    static let checkout = "0198f2c0-0000-7000-8000-00000000c001"
    static let webhooks = "0198f2c0-0000-7000-8000-00000000c002"
    static let scratch = "0198f2c0-0000-7000-8000-00000000c003"
    /// `-phone-loose-hidden`: Billing's spike, owned by no card.
    static let spike = "0198f2c0-0000-7000-8000-00000000c004"
    static let mainOrchestrator = "0198f2c0-0000-7000-8000-00000000d001"
    static let agent = "0198f2c0-0000-7000-8000-00000000d002"
    static let shell = "0198f2c0-0000-7000-8000-00000000d003"
    static let billingOrchestrator = "0198f2c0-0000-7000-8000-00000000d004"
    /// `-phone-rep`: Bob's rep in Billing (ov-526).
    static let bobsRep = "0198f2c0-0000-7000-8000-00000000d005"
    /// The draft `-phone-draft-held` holds behind a dialog (ov-385).
    static let heldDraft = "0198f2c0-0000-7000-8000-00000000f385"
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

    /// Whether Main has its orchestrator: yes, unless `-phone-no-orchestrators`.
    private static var mainLed: Bool { !CommandLine.arguments.contains("-phone-no-orchestrators") }
    /// `-phone-native`: Main's orchestrator serves its conversation.
    private static var nativeOrchestrator: Bool { CommandLine.arguments.contains("-phone-native") }
    /// The subagents Main's orchestrator has running, as the fleet names
    /// them (ov-619): `-native-agents`' four, under `-phone-native`, each
    /// under its row's call id (`a1`...`a4`).
    private static var orchestratorSubagents: [String]? {
        guard nativeOrchestrator, CommandLine.arguments.contains("-native-agents") else { return nil }
        // What each was asked, as the runner names them (`watch.rs`).
        return [
            "ov-452 conversation view hierarchy", "ov-454 image paste and attachments", "ov-453 subagent tray",
            "ov-455 queue phrasing",
        ]
    }
    /// `-native-agents-after-turn`: the turn that started those four has
    /// ended, so the runner names none of them and only counts them.
    private static var afterTurn: Bool { CommandLine.arguments.contains("-native-agents-after-turn") }
    /// What answers the conversation's calls under `-phone-native`.
    private let native = NativeHarnessRunner()

    private let connection: Connection
    /// Whether Billing has an orchestrator yet. It starts without one,
    /// unless `-phone-billing-led`.
    private var billingLed =
        CommandLine.arguments.contains("-phone-billing-led") || HarnessRunner.startStates
    /// The draft held on Billing's orchestrator, as a fleet row carries it.
    private var billingHold: [String: Any]?
    /// Whether fc-3-webhooks is put away, as the runner keeps it: starts so
    /// under `-phone-webhooks-hidden`, and `worktree.unhide` clears it.
    private var webhooksHidden = CommandLine.arguments.contains("-phone-webhooks-hidden")
    /// Whether spike is put away: starts so under `-phone-loose-hidden`.
    private var spikeHidden = true
    /// Whether a started orchestrator's pane has ended at once, exit 127
    /// (`-phone-start-127`).
    private var billingDead = false
    /// `-phone-rep`: whether Bob's rep is running, stopped or not yet made.
    private enum RepState { case none, running, stopped }
    private var repState: RepState =
        CommandLine.arguments.contains("-phone-rep-running") ? .running
        : CommandLine.arguments.contains("-phone-rep-stopped") ? .stopped : .none
    private static var repLong: Bool { CommandLine.arguments.contains("-phone-rep-long") }
    /// `-phone-claude`: Bob's own Claude login, as the runner would say it.
    private var claudeState: String =
        CommandLine.arguments.contains("-phone-claude-signed-in") ? "signed_in"
        : CommandLine.arguments.contains("-phone-claude-stopped") || CommandLine.arguments.contains("-phone-claude-line")
        ? "stopped_working" : "none"
    /// `-phone-own-logins`: which agents need each member's own account.
    private var ownLoginSwitches: [String: Bool] = ["claude": false, "codex": true, "cursor": false]
    private var claudeEmail: String {
        Self.repLong ? "bob.montgomery-fitzgerald@platform-infrastructure.example-corporation.com" : "bob@claude.example"
    }
    /// The items still waiting, by id.
    private var waiting: [String]
    /// Every write the screens made, as `harness-sent` shows it:
    /// `task.note <task> <body>`, `start <workspace> <harness> replace=<b>`.
    private(set) var sent: [String] = []

    /// A link the app handed to the system (`openURL`), in `sent` as `opened <url>`:
    /// what a test asserts, in place of waiting for another app to come forward.
    func opened(_ url: URL) { sent.append("opened \(url.absoluteString)") }
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

    /// A runner notice, as the client core would queue it: the plan it
    /// answers with changes first, as the runner's would have.
    func deliver(_ notice: [String: Any]) async {
        HarnessPlan.version += 1
        await connection.hearNews(notice)
    }

    /// The link going quiet (`HarnessTaps.linkLost`).
    func loseLink() { connection.standInLinkLost() }

    func stand() async {
        // Who this phone remembers it is (`TalkSeats`): forgotten at each launch,
        // unless a test asks to carry it over.
        if !CommandLine.arguments.contains("-phone-rep-keep-you") {
            UserDefaults.standard.removeObject(forKey: "talk.you.\(Self.host.id.uuidString)")
        }
        connection.adoptFocusMemory(runner: Self.host.id.uuidString)
        connection.standInCalls = { [weak self] method, args in
            guard let self else { throw ClientCore.CoreError.notStarted }
            return try await self.answer(method, args)
        }
        // Calls made on the core itself, as a terminal pane's screen reads are.
        nonisolated(unsafe) let calls = connection.standInCalls
        await connection.core.standIn { method, args in
            guard let calls else { throw ClientCore.CoreError.notStarted }
            return try await calls(method, args)
        }
        connection.standIn(host: Self.host)
        connection.standIn(
            on: fleet(),
            repositories: CommandLine.arguments.contains("-phone-no-repositories")
                ? []
                : [
                    Repository(
                        id: Self.repository, short: "a001", displayName: "overnight", remote: "")
                ] + HarnessManyRepos.repositories,
            build: DaemonBuild(
                version: "harness", matches: true, platform: "harness",
                capabilities: Set(
                    ["tasks", "needs_you", "workstreams", "terminal_task"]
                        + (Self.keepsReads ? ["board_reads"] : [])
                        + (CommandLine.arguments.contains("-phone-files-old") ? [] : ["worktree_files", "read_only_folders"])
                        + (HarnessPlan.advertised ? ["board_plan"] : [])
                        + (HarnessPlan.pagesAdvertised ? ["board_pages"] : [])
                        + (HarnessPlan.rulingsAdvertised ? ["board_rulings", "board_ruling_actions"] : [])
                        + (HarnessPlan.agentsAdvertised ? ["board_agents"] : [])
                        + (PhonePrHarness.advertised ? ["pr_actions"] : [])
                        + (CommandLine.arguments.contains("-phone-usage-old") ? [] : ["agent_usage"])
                        + (Self.shared ? ["representatives"] : [])
                        + (Self.nativeOrchestrator ? ["agent", "agent_rows", "agent_compose", "compose", "terminal_interrupt"] : [])
                        + (Self.nativeOrchestrator && CommandLine.arguments.contains("-native-agents") ? ["subagent_rows"] : [])
                        + (Self.claudeSignIn ? ["claude_sign_in"] : [])
                        + (Self.ownLogins ? ["member_own_logins"] : [])),
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

    /// `-phone-shared`: see the launch arguments above.
    static var shared: Bool {
        CommandLine.arguments.contains("-phone-shared") || CommandLine.arguments.contains("-phone-rep") || claudeSignIn
    }

    /// The owner's own phone: `you` is empty.
    static var ownerPhone: Bool {
        CommandLine.arguments.contains("-phone-rep-owner") || CommandLine.arguments.contains("-phone-claude-owner")
    }

    /// `-phone-claude`: see the launch arguments above.
    static var claudeSignIn: Bool { CommandLine.arguments.contains("-phone-claude") }

    /// `-phone-own-logins`: see the launch arguments above.
    static var ownLogins: Bool { CommandLine.arguments.contains("-phone-own-logins") }

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
        var checkoutTerminals =
            Self.mainLed
            ? [
                Terminal(
                    id: Self.mainOrchestrator, short: "d001", title: "claude", preset: "claude",
                    runningAgent: Self.nativeOrchestrator ? "claude" : nil, state: "running", activity: "working", activitySince: now - 60_000,
                    subagents: Self.afterTurn ? [] : Self.orchestratorSubagents,
                    subagentsRunning: Self.orchestratorSubagents?.count,
                    subagentIds: Self.afterTurn ? [] : Self.orchestratorSubagents.map { _ in ["a1", "a2", "a3", "a4"] },
                    epoch: 1, paneMode: "terminal",
                    workspace: Self.main,
                    role: "orchestrator")
            ] : []
        if billingLed {
            checkoutTerminals.append(
                Terminal(
                    id: Self.billingOrchestrator, short: "d004", title: "claude",
                    preset: "claude", state: billingDead ? "exited" : "running",
                    exitCode: billingDead ? 127 : nil, activity: "idle",
                    draftHold: DraftHold(json: billingHold), epoch: 1,
                    paneMode: "terminal", workspace: Self.billing,
                    role: "orchestrator"))
        }
        if repState != .none {
            checkoutTerminals.append(
                Terminal(
                    id: Self.bobsRep, short: "d005", title: "Bob’s rep", preset: "claude",
                    state: repState == .running ? "running" : "exited",
                    exitCode: repState == .running ? nil : 0, activity: "idle",
                    epoch: 1, paneMode: "terminal", workspace: Self.billing, role: "agent"))
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
                            taskId: Self.agentTask, workspace: Self.billing,
                            role: "agent"),
                        Terminal(
                            id: Self.shell, short: "d003", title: "Terminal 1", preset: "shell",
                            state: "running", epoch: 1, paneMode: "terminal",
                            workspace: Self.billing, role: "shell"),
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
            ] + (CommandLine.arguments.contains("-phone-loose-hidden")
                ? [
                    Worktree(
                        id: Self.spike, short: "c004", repository: Self.repository,
                        task: "spike", branch: "spike", state: spikeHidden ? "hidden" : "ready", terminals: [],
                        workspace: Self.billing, openTasks: [])
                ] : []) + HarnessManyRepos.worktrees,
            workspaces: [
                WorkspaceSummary(
                    id: Self.main, name: "Main", taskPrefix: "ove", isMain: true, ordinal: 0,
                    repository: Self.repository, orchestrator: Self.mainLed ? Self.mainOrchestrator : nil,
                    lead: Self.shared ? "" : nil),
                WorkspaceSummary(
                    id: Self.billing,
                    name: Self.repLong ? "Billing and Collections for the Enterprise Platform Team" : "Billing",
                    taskPrefix: "bil", isMain: false,
                    ordinal: 1, repository: Self.repository,
                    orchestrator: billingLed ? Self.billingOrchestrator : nil,
                    lead: Self.shared
                        ? (CommandLine.arguments.contains("-phone-rep-lead") ? "bob" : "alexandra") : nil),
            ] + HarnessManyRepos.workspaces)
    }

    /// A member's `claude_login` as `member.list` carries it (`-phone-claude`):
    /// the email only for the member's own phone and the owner's, as the runner tells it.
    private func claudeLogin(of handle: String) -> [String: Any] {
        guard Self.claudeSignIn else { return [:] }
        let mine = handle == "bob"
        let state = mine ? claudeState : "signed_in"
        let told = mine || Self.ownerPhone
        let email = mine ? claudeEmail : "alexandra@claude.example"
        return ["claude_login": ["state": state, "email": told && state != "none" ? email : ""]]
    }

    /// `member.list`'s `own_logins` (`-phone-own-logins`), or nothing.
    private func ownLoginsJSON() -> [String: Any] {
        guard Self.ownLogins else { return [:] }
        return ["own_logins": ownLoginSwitches.merging(["may_change": Self.ownerPhone]) { $1 }]
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
        case "task.get" where CommandLine.arguments.contains("-phone-member-notes"):
            guard args["task"] as? String == Self.decisionTask else { return try json(["notes": []]) }
            return try json(["notes": [question] + membersNotes])
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
                let text = args["text"] as? String,
                text.hasPrefix("About bil-") || text.hasPrefix("About ruling ") || text.hasPrefix("Please reverse ruling ")
            else { throw ClientCore.CoreError.rejected("bad draft", word: "invalid-argument") }
            if CommandLine.arguments.contains("-phone-draft-refused") {
                throw ClientCore.CoreError.rejected("not safe to paste", word: "agent-not-connected")
            }
            sent.append("draft billing \(text)")
            // `-phone-draft-held` (ov-385): a dialog is up in the pane, so the
            // runner holds the draft and says so on the pane.
            if CommandLine.arguments.contains("-phone-draft-held") {
                billingHold = ["id": Self.heldDraft, "state": "waiting", "heldMs": 0, "expiresMs": 1_800_000, "endedMs": 0]
                connection.standIn(on: fleet())
                return try json(["held": billingHold as Any])
            }
            return try json([:])
        case "terminal.draft_withdraw":
            guard args["terminal"] as? String == Self.billingOrchestrator, args["hold"] as? String == Self.heldDraft
            else { throw ClientCore.CoreError.rejected("no such hold", word: "not-found") }
            sent.append("withdraw billing")
            billingHold?["state"] = "withdrawn"
            connection.standIn(on: fleet())
            return try json(["hold": billingHold as Any])
        case "worktree.list_dir", "worktree.read_file":
            return try files(method, args)
        case "terminal.screen" where Self.showsTerminalKey && args["terminal"] as? String == Self.shell:
            // One screen, the way the runner answers a capture: base64 of the
            // bytes that draw it. A key in a sentence, and a version beside it
            // that must stay plain.
            // `-phone-terminal-alt`: the same screen drawn by a full-screen
            // program, on the alternate screen with mouse reporting on, as
            // vim or htop draws (review 1 of ov-609).
            let program = Self.alternateScreen ? "\u{1b}[?1049h\u{1b}[?1000h\u{1b}[H" : ""
            let text = program + "bil-9 is done; build finished (v1.2, bil-1.2)\r\n$ "
            return try json([
                "contents": Data(text.utf8).base64EncodedString(), "columns": 52, "rows": 12,
                "cursorColumn": 2, "cursorRow": 1, "revision": 1, "unchanged": false,
            ])
        case "terminal.write" where Self.alternateScreen:
            // What reached the program, for a test that nothing should.
            sent.append("write \((args["hex"] as? String) ?? "")")
            return try json([:])
        case "worktree.hide", "worktree.unhide":
            if CommandLine.arguments.contains("-phone-hide-fails") {
                throw ClientCore.CoreError.rejected("unavailable", word: "unavailable")
            }
            switch args["worktree"] as? String {
            case Self.webhooks:
                webhooksHidden = method == "worktree.hide"
                sent.append("\(method) fc-3-webhooks")
            case Self.spike where CommandLine.arguments.contains("-phone-loose-hidden"):
                spikeHidden = method == "worktree.hide"
                sent.append("\(method) spike")
            default:
                throw ClientCore.CoreError.rejected("bad worktree", word: "invalid-argument")
            }
            connection.standIn(on: fleet())
            return try json([:])
        case "rep.start" where Self.shared:
            // A member starts their own rep (ov-526): three seconds, as a Claude
            // Code pane takes, so "Starting Your Rep…" is on screen long enough to read.
            guard args["workspace"] as? String == Self.billing, args["handle"] as? String == "bob"
            else { throw ClientCore.CoreError.rejected("bad start", word: "invalid-argument") }
            sent.append("rep.start bob billing")
            try? await Task.sleep(for: .seconds(3))
            if CommandLine.arguments.contains("-phone-rep-refused") {
                throw ClientCore.CoreError.rejected("not yours", word: "permission-denied")
            }
            repState = .running
            connection.standIn(on: fleet())
            if CommandLine.arguments.contains("-phone-rep-raced") {
                // Another device started it a moment ago.
                throw ClientCore.CoreError.rejected("conflict", word: "resource-conflict", what: "rep_running")
            }
            return try json(["id": Self.bobsRep, "desk": "bil-1", "claude_account": "alice@example.com"])
        case "host.health":
            // `-phone-claude-key-warning`: the owner's shell exports all three
            // Claude variables (ov-651); the sentence the client library words.
            let warning = "Your shell’s startup files set ANTHROPIC_API_KEY, CLAUDE_CODE_OAUTH_TOKEN and ANTHROPIC_AUTH_TOKEN, so a member typing claude in their own shell would use your account."
            return try json([
                "platform": "darwin-aarch64", "daemonVersion": "0.9.0", "protocolVersion": 1, "healthy": true, "reasons": [String](),
                "livePanes": 3, "claudeKeyWarning": (CommandLine.arguments.contains("-phone-claude-key-warning") ? warning : NSNull()) as Any,
            ])
        case "member.list" where Self.shared:
            sent.append(method)
            if CommandLine.arguments.contains("-phone-rep-list-fails") {
                throw ClientCore.CoreError.rejected("unavailable", word: "unavailable")
            }
            if CommandLine.arguments.contains("-phone-rep-slow-list") { try? await Task.sleep(for: .seconds(8)) }
            let rep = { (pane: String, board: String) in ["terminal": pane, "workspace": board] }
            return try json([
                "members": [
                    [
                        "handle": "alexandra", "name": "Alexandra Montgomery-Fitzgerald of the Platform Team",
                        "devices": ["phone-a"], "reps": 2,
                        "rep_panes": [rep("r1", Self.main), rep("r2", Self.billing)],
                    ].merging(claudeLogin(of: "alexandra")) { $1 },
                    [
                        "handle": "bob", "name": "Bob", "devices": ["phone-b"],
                        // Added by QR as a teammate (ov-525): his directory
                        // account, and the email People shows.
                        "account": "user_bob",
                        "email": Self.repLong
                            ? "bob.montgomery-fitzgerald@platform-infrastructure.example-corporation.com"
                            : "bob@example.com",
                        "reps": repState == .none ? 0 : 1,
                        "rep_panes": repState == .none ? [] : [rep(Self.bobsRep, Self.billing)],
                    ].merging(claudeLogin(of: "bob")) { $1 },
                ],
                "claude_account": Self.repLong
                    ? "platform-team-shared-account@example-corporation.com" : "alice@example.com",
                "you": Self.ownerPhone ? "" : "bob",
            ].merging(ownLoginsJSON()) { $1 })
        case "member.set_own_login" where Self.ownLogins:
            guard Self.ownerPhone, !CommandLine.arguments.contains("-phone-own-logins-refused"),
                let harness = args["harness"] as? String, let on = args["on"] as? Bool
            else {
                throw ClientCore.CoreError.rejected("scope denied: host_admin required", word: "scope-denied")
            }
            sent.append("own-login \(harness) \(on)")
            ownLoginSwitches[harness] = on
            return try await answer("member.list", [:])
        case "claude.sign_in" where Self.claudeSignIn:
            // A link takes a moment, as claude printing it does (ov-584).
            try? await Task.sleep(for: .seconds(CommandLine.arguments.contains("-phone-claude-slow") ? 6 : 0.4))
            guard args["handle"] as? String == "bob"
            else { throw ClientCore.CoreError.rejected("not yours", word: "not-yours", what: "not_yours") }
            if CommandLine.arguments.contains("-phone-claude-too-often") {
                throw ClientCore.CoreError.rejected("too many", word: "resource-conflict", what: "sign_in_too_often")
            }
            sent.append("claude.sign_in bob")
            return try json([
                "session": "0198f2c0-0000-7000-8000-00000000c0de",
                "url": "https://claude.com/cai/oauth/authorize?code=true&client_id=harness&state=abc",
                "expires_at": Int64(Date().timeIntervalSince1970 * 1000) + 600_000,
            ])
        case "claude.sign_in_finish" where Self.claudeSignIn:
            // Never the code itself: its length is all a test needs.
            let code = args["code"] as? String ?? ""
            sent.append("claude.sign_in_finish length=\(code.count)")
            try? await Task.sleep(for: .seconds(1))
            if CommandLine.arguments.contains("-phone-claude-expired") {
                throw ClientCore.CoreError.rejected("gone", word: "not-found")
            }
            if code.hasPrefix("refused") {
                throw ClientCore.CoreError.rejected("refused", word: "resource-conflict", what: "code_refused")
            }
            claudeState = "signed_in"
            return try json(["state": claudeState, "email": claudeEmail])
        case "claude.sign_in_cancel" where Self.claudeSignIn:
            sent.append("claude.sign_in_cancel")
            return try json(["state": claudeState, "email": claudeState == "none" ? "" : claudeEmail])
        case "claude.sign_out" where Self.claudeSignIn:
            sent.append("claude.sign_out \(args["handle"] as? String ?? "")")
            claudeState = "none"
            return try json(["state": "none", "email": ""])
        case "plan.get", "plan.events", "page.list", "ruling.keep", "ruling.keep_all":
            guard let plan = HarnessPlan(),
                let data = try await plan.answer(method, args, boards: [Self.main, Self.billing])
            else { throw ClientCore.CoreError.rejected("not in the harness", word: "unimplemented") }
            sent.append(
                method == "plan.events"
                    ? "plan.events \((args["theme"] as? String) ?? (args["lane"] as? String) ?? "")"
                    : (method.hasPrefix("ruling.") ? "\(method) \((args["ruling"] as? String) ?? "")" : method))
            return data
        case "pr.ready", "pr.thread_reply":
            guard PhonePrHarness.advertised else {
                throw ClientCore.CoreError.rejected("not in the harness", word: "unimplemented")
            }
            let (data, line) = try PhonePrHarness.answer(method, args, plan: HarnessPlan())
            sent.append(line)
            return data
        default:
            if Self.nativeOrchestrator, method.hasPrefix("agent.") || method.hasPrefix("terminal.") {
                return try await native.answer(method, args)
            }
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

    /// `-phone-terminal-key`.
    static var showsTerminalKey: Bool { CommandLine.arguments.contains("-phone-terminal-key") }
    /// `-phone-terminal-alt`: see `terminal.screen`.
    static var alternateScreen: Bool { CommandLine.arguments.contains("-phone-terminal-alt") }

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
            "extra": ["options": Self.questionOptions],
        ]
    }

    /// bil-7's options: two short ones, or with `-phone-long-options` three
    /// whose text runs past a phone's width (ov-431).
    static var questionOptions: [String] {
        // `-phone-free-answer`: no options, so the card offers Answer… and a text field.
        if CommandLine.arguments.contains("-phone-free-answer") { return [] }
        return CommandLine.arguments.contains("-phone-long-options")
            ? [
                "pdfkit",
                "wkhtmltopdf, run as a separate process the daemon starts for each export and stops once the file is written",
                "Render the invoice on the server with a headless browser, and keep the layout in one template shared with the web",
            ] : ["pdfkit", "wkhtmltopdf"]
    }

    private var answered: [String: Any] {
        [
            "id": "note-a", "kind": "answer", "actor": "user",
            "at": Int64(Date().timeIntervalSince1970 * 1000), "body": "pdfkit",
        ]
    }

    /// `-phone-member-notes` (ov-707): bil-7 answered by Alexandra, a member,
    /// then a comment from the owner and one from Bob, the member this phone
    /// is unless it's the owner's (`-phone-rep-owner`).
    private var membersNotes: [[String: Any]] {
        let now = Int64(Date().timeIntervalSince1970 * 1000)
        return [
            ["id": "note-a", "kind": "answer", "actor": "member:alexandra", "at": now - 300_000, "body": "pdfkit"],
            [
                "id": "note-o", "kind": "comment", "actor": "user", "at": now - 200_000,
                "body": "Hold the export until the rounding is fixed.",
            ],
            [
                "id": "note-b", "kind": "comment", "actor": "member:bob", "at": now - 100_000,
                "body": "The totals row is off by a cent on multi-currency invoices.",
            ],
        ]
    }

    /// `-phone-team`'s list: whose each item is, as a member's phone is told
    /// it, and the owner's orchestrator waiting on them.
    private static func teamItems(_ items: [[String: Any]], now: Int64) -> [[String: Any]] {
        func concern(_ handle: String, _ name: String, yours: Bool) -> [String: Any] {
            ["handle": handle, "name": name, "yours": yours]
        }
        var out = items.map { item -> [String: Any] in
            var item = item
            switch item["kind"] as? String {
            case "ask": item["concern"] = concern("alice", "Alice Chen-Montgomery", yours: true)
            case "decision": item["concern"] = concern("bob", "Bartholomew Okafor-Lindqvist", yours: false)
            default: break
            }
            return item
        }
        out.append([
            "id": "blocked:\(mainOrchestrator)", "kind": "blocked", "rank": 100_000_900, "since": now - 900_000,
            "workspace_id": main, "workspace_name": "Main", "repository_id": repository,
            "terminal": [
                "id": mainOrchestrator, "worktree_id": checkout, "label": "claude",
                "role": "orchestrator", "pane_mode": "terminal",
            ],
            "question": "Ship the migration to staging tonight, or wait for Monday’s review?",
            "actions": [["id": "open", "title": "Open", "destructive": false, "primary": false]],
            "concern": concern("", "", yours: false),
        ])
        return out
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
                    "role": "agent", "pane_mode": "terminal",
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
                "actions": Self.questionOptions.map {
                    ["id": $0, "title": $0, "destructive": false, "primary": false] as [String: Any]
                },
            ])
        }
        out += PhonePrHarness.items()
        out += HarnessManyRepos.items(now: now)
        if CommandLine.arguments.contains("-phone-team") { out = Self.teamItems(out, now: now) }
        if CommandLine.arguments.contains("-phone-claude-line") {
            // Bob's own line: the runner words it for him and offers the one button.
            out.append([
                "id": "claude-login:bob", "kind": "claude_login", "rank": 100_000_300, "since": now - 300_000,
                "workspace_name": "", "question": "Your Claude login on Studio stopped working.",
                "actions": [["id": "sign_in", "title": "Sign In Again", "destructive": false, "primary": true]],
            ])
        }
        // Below Control, the runner sends an item's id, kind, rank, subject
        // and a fixed question, and none of its answers (spec §2.4).
        guard Self.readOnly else { return out }
        return out.map { item in
            var redacted = item
            // A login line names a person and its button is answered at any scope: kept whole.
            if item["kind"] as? String == "claude_login" { return item }
            for key in ["actions", "detail", "ask_id", "link"] { redacted.removeValue(forKey: key) }
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
        return startStates() + HarnessPlan.boardTasks(on: Self.billing) + [
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
