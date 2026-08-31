import SwiftUI

#if DEBUG

// The navigation shell over canned fleets, so it can be driven before it is
// wired to anything.
//
// `-shell-harness`, alongside `-agent-layout-harness` and
// `-changes-layout-harness` in `FarCoolerApp.swift`, and for the same reason
// those two exist: a surface that has only ever been argued about from the
// code is a surface nobody has looked at. This one has a second reason as
// well — the shell is a GESTURE, and a gesture cannot be reviewed in a
// screenshot at all. It has to be swiped, and swiping it needs a fleet, and a
// fleet needs a runner, a daemon and some agents actually doing something.
//
// So: three canned fleets, one flag each, and the shell is real from the first
// commit. `ShellGestureTests` drives this same harness, which is the only way
// the axis lock and the commit threshold get a test at all.
//
// ## The three scales, and why 40 is not a stress test
//
// `-shell-4` is the common case, `-shell-10` a busy afternoon, and `-shell-40`
// is the requirement that killed every other design. A bar that shows the
// workspaces as a strip has to compress to fit them, and at forty a compressed
// strip is forty illegible slivers — which is why the bar here shows exactly
// ONE workspace and the fleet lives in the overview. 40 is therefore not an
// edge case to survive; it is the case the design was chosen for, and a change
// to the shell that has not been looked at with `-shell-40` has not been
// looked at.

/// The shell, standing on a fixture.
struct ShellHarness: View {
    static var isRequested: Bool {
        CommandLine.arguments.contains("-shell-harness")
    }

    /// The one thing a canned `ChangesView` needs, and the same one
    /// `ChangesLayoutHarness` stands it on: a connection nobody connects, for
    /// the sake of the store hanging off it.
    @StateObject private var connection = Connection()

    /// A card on another runner, tapped. The harness cannot actually cross —
    /// there is no `RunnerStore` behind a fixture and nothing to connect to —
    /// but the ALERT is the half worth driving, because the wording is what
    /// somebody reads before they lose a scrollback and a fixture cannot
    /// contradict it.
    @State private var crossing: ShellCrossing?

    var body: some View {
        let fleet = Self.fleet
        ZStack {
            // A ground for the glass to be glass against. The panes are text
            // on nothing, and glass over nothing has no material to sample —
            // the bar would read as a grey rectangle and every screenshot of
            // it would be a screenshot of the wrong thing.
            LinearGradient(
                colors: [Color(red: 0.06, green: 0.07, blue: 0.09), Color(red: 0.02, green: 0.02, blue: 0.03)],
                startPoint: .top, endPoint: .bottom)
                .ignoresSafeArea()

            ShellRootView(
                fleet: fleet,
                initial: ShellPosition(workspace: 0, tab: 0),
                // `-shell-overview` lands straight on the all-workspaces view.
                // Reaching it otherwise takes a drag past the last row, which
                // is exactly the state a screenshot most wants and a script
                // least reliably produces.
                openingOnOverview: CommandLine.arguments.contains("-shell-overview"),
                liveServer: "this-mac",
                elsewhere: Self.elsewhere,
                onCross: { group, workspace in
                    crossing = ShellCrossing(group: group, workspace: workspace)
                }
            ) { slot in
                ShellPanePlaceholder(slot: slot, changes: changesStore)
            }
        }
        .shellCrossingAlert($crossing, leaving: "this-mac") { _ in }
        .preferredColorScheme(.dark)
    }

    /// The review pane over `ChangesLayoutHarness`'s own canned change set, or
    /// nil when this launch did not ask for one.
    ///
    /// `-shell-changes`, and it is the fixture the shell's page turn most
    /// needed and did not have. A diff is not one scroll view: every hunk is a
    /// horizontal `ScrollView` of its own, because a diff line is a line and
    /// wrapping one breaks the only property a diff has. Nothing else in this
    /// app puts a horizontal scroller inside a pane, and a shell tested only
    /// against panes that scroll vertically is a shell that has never met the
    /// gesture it actually loses. See `ShellPaneScrollTests`.
    ///
    /// The SAME canned set `-changes-layout-harness` stands on, so the two
    /// harnesses cannot come to disagree about what a diff looks like.
    private var changesStore: ChangesStore? {
        guard CommandLine.arguments.contains("-shell-changes") else { return nil }
        let store = connection.changesStores.store(for: "harness")
        ChangesLayoutHarness.standIn(store)
        return store
    }

    /// Which fixture this launch asked for.
    ///
    /// One BARE flag each — `-shell-40` — rather than `-shell 40`. The pair
    /// form works under `simctl launch` and silently does nothing under
    /// `XCUIApplication.launchArguments`, which is the trap `AgentLayoutHarness`
    /// documents at length; a harness whose flags only work from one of the two
    /// launchers is a harness the tests cannot use.
    static var fleet: ShellFleet {
        let args = CommandLine.arguments
        if args.contains("-shell-4") { return canned(count: 4) }
        if args.contains("-shell-40") { return canned(count: 40) }
        return canned(count: 10)
    }

    /// Whether this launch asked for some of the fleet to be put away.
    private static var hides: Bool {
        CommandLine.arguments.contains("-shell-hidden")
    }

    /// `count` workspaces, with tab counts and states that vary the way a real
    /// fleet's do.
    ///
    /// Deliberately not `count` identical workspaces. The three things this
    /// fixture has to be able to show wrong are the crossing rule (a swipe off
    /// the end of a workspace), the ribbon (four different marks side by side)
    /// and the overview sort (precedence lifting the loud ones) — and a fleet
    /// where every workspace has two working tabs shows none of them. The
    /// numbers come off the index so the fixture is reproducible: the same
    /// flag always produces the same fleet, which is what makes a screenshot
    /// comparable to the last one.
    static func canned(count: Int) -> ShellFleet {
        ShellFleet(
            workspaces: (0..<count).map { index in
                // Tab counts that vary, and NOT in ascending order: a fleet
                // whose workspaces get steadily bigger reads as a pattern, and
                // a fixture that looks designed stops being a stand-in for one
                // that is not. Three at the front because that is the size the
                // column and the overview threshold are usually looked at, and
                // a one-tab workspace in the middle because the column's own
                // ends and the overview's reach both depend on the count and
                // both have to be reachable by flag alone.
                let tabs = [3, 2, 5, 1, 4][index % 5]
                return ShellWorkspace(
                    id: "ws-\(index)",
                    name: Self.names[index % Self.names.count]
                        + (index >= Self.names.count ? "-\(index / Self.names.count + 1)" : ""),
                    // A server on some of them and not others, so the bar's
                    // both-ways case is reachable by flag: the local runner
                    // says nothing, the rest name themselves.
                    //
                    // **Except when the grid has real sections.** These
                    // workspaces are the LIVE fleet, and a live card that
                    // names `eu-runner-1` while sitting under a `this-mac`
                    // heading is a fixture contradicting itself — which in the
                    // app cannot happen, because `ShellFleetMap.of` leaves
                    // `server` nil for every workspace on the runner it is
                    // connected to. So the bar's fixture and the grid's
                    // fixture take turns.
                    server: CommandLine.arguments.contains("-shell-servers") || index % 3 == 0
                        ? nil : Self.servers[index % Self.servers.count],
                    // FIVE tails against a four-workspace mark cycle, and the
                    // count is the point rather than the stride.
                    //
                    // `index % 4` put an identical tail on every card in a
                    // precedence group, and the overview sorts BY precedence —
                    // so the grid read as forty cards all saying one sentence.
                    // A different stride does not fix that: any linear
                    // function of `index` is CONSTANT within a residue class
                    // mod 4, so as long as the two cycles share a length the
                    // tail is a restatement of the mark. Five and four are
                    // coprime, so the pair walks all twenty combinations.
                    tail: Self.tails[index % Self.tails.count],
                    // `-shell-hidden`, and every fifth one rather than every
                    // second: the section has to be a MINORITY of the grid or
                    // the fixture stops looking like a fleet somebody put a
                    // few things away in. Coprime with neither cycle above, so
                    // it does not line up with the marks or the tails.
                    isHidden: Self.hides && index % 5 == 3,
                    tabs: (0..<tabs).map { tab in
                        ShellTab(
                            id: "ws-\(index)-tab-\(tab)",
                            title: tab == 0 ? "Diff" : Self.agents[tab % Self.agents.count],
                            // Tab 0 is the diff, in every workspace. "`Diff` is
                            // a tab like any other, first in the list" — and
                            // only a diff tab is ever `unreadDiff`, which is a
                            // model rule rather than a style choice. A fixture
                            // that put the diff last would also put its cyan
                            // ring at the wrong end of every ribbon, and a
                            // ribbon is only learnable because the marks do
                            // not move.
                            mark: mark(workspace: index, tab: tab).mark,
                            wantsAttention: mark(workspace: index, tab: tab).wantsAttention)
                    })
            })
    }

    /// Every state the bar can draw, spread so that each one is on screen at
    /// the scales a person looks at.
    ///
    /// **Six now rather than four**, because the mark gained the axis it was
    /// missing: a producing agent and an idle one are different drawings, and
    /// a fixture that cannot produce both is a fixture in which the defect
    /// this replaced would still not be visible.
    ///
    /// **Nothing asserts that every arm below is reachable**, and the paragraph
    /// after this one is what that costs — the app has no unit-test target, so
    /// this file is checked by being looked at. `-shell` with enough workspaces
    /// is the check: the cycles are `% 4` on the diff and `% 5` on the agents
    /// against tab counts of `[3, 2, 5, 1, 4]`, so ten workspaces show every
    /// arm. If an arm is ever made unreachable again it will go unnoticed the
    /// same way, which is an argument for the target and not for a comment.
    ///
    /// Tab 0 is the diff, so "the first AGENT" is tab 1. This read `tab == 0`
    /// while the fixture put the diff last, and moving the diff to the front
    /// left that arm unreachable — every workspace would have quietly
    /// rendered as working, and the amber mark the whole ribbon exists for
    /// would have been absent from every screenshot.
    static func mark(workspace: Int, tab: Int) -> (mark: GlanceMark, wantsAttention: Bool) {
        // The diff, which is never an agent and so never states a core.
        if tab == 0 {
            return (
                GlanceMark(attention: workspace % 4 == 0 ? .toReview : .quiet, core: nil), false
            )
        }
        // Five agent states over a cycle coprime with neither the tab counts
        // (period 5 — hence the `+ tab`, which walks the residues) nor the
        // four-workspace diff cycle above.
        switch (workspace + tab) % 5 {
        case 0: return (GlanceMark(attention: .needsYou, core: .atAPrompt), true)
        // Done: the review tier, and it wants you even though it does not draw
        // the amber ring. See `ShellTab.wantsAttention`.
        case 1: return (GlanceMark(attention: .toReview, core: .atAPrompt), true)
        case 2: return (GlanceMark(attention: .quiet, core: .producing), false)
        case 3: return (GlanceMark(attention: .quiet, core: .atAPrompt), false)
        default:
            return (GlanceMark(attention: .quiet, core: .producing, link: .broken), false)
        }
    }

    private static let servers = ["gpu-box-2", "eu-runner-1"]

    /// Two OTHER runners, as the app would have last seen them.
    ///
    /// `-shell-servers`, behind a flag rather than always on, for the reason
    /// every flag in this harness is: it changes what the grid CONTAINS, and
    /// the tests that were written against a one-runner grid are still the
    /// tests for a one-runner grid. A fixture that quietly grew two extra
    /// sections would have rewritten all of them.
    ///
    /// The marks are the interesting part and they are not uniform. A cached
    /// runner is not uniformly grey: `RunnerDirectory.decayed` holds
    /// `needsYou` and `unreadDiff` at any age and lets `working` go dashed, so
    /// this fixture carries one of each — a fixture where every cached ring
    /// was dashed could not show that rule working or failing.
    ///
    /// One of them is HIDDEN, which must not be drawn at all: the way back
    /// from hiding is on the runner the worktree is on, and this is not that
    /// runner.
    static var elsewhere: [ShellServerGroup] {
        guard CommandLine.arguments.contains("-shell-servers") else { return [] }
        let now = Date()
        return [
            RunnerDirectory(
                runner: "runner-gpu", label: "gpu-box-2",
                seenAt: now.addingTimeInterval(-2 * 60 * 60),
                workspaces: [
                    directory("fix/token-refresh", mark: "needsYou"),
                    directory("feat/queue-drain", mark: "working"),
                    directory("chore/put-away", mark: "working", isHidden: true),
                ]
            ).group(),
            RunnerDirectory(
                runner: "runner-eu", label: "eu-runner-1",
                seenAt: now.addingTimeInterval(-9 * 60),
                workspaces: [directory("spike/watch-sync", mark: "unreadDiff")]
            ).group(),
        ]
    }

    private static func directory(
        _ name: String, mark: String, isHidden: Bool = false
    ) -> RunnerDirectory.Workspace {
        RunnerDirectory.Workspace(
            id: name, name: name, isHidden: isHidden,
            tabs: [
                RunnerDirectory.Tab(title: "Diff", mark: mark),
                RunnerDirectory.Tab(title: "claude", mark: "working"),
            ],
            tail: ["$ npm test", "142 passing, 0 failing"])
    }

    /// What a card's terminal last said. Four shapes rather than one, because
    /// a grid where every card says the same thing cannot show whether the
    /// tail is doing its job — and one of them is a question, which is the
    /// case the amber mark exists for and the one a person is scanning for.
    private static let tails: [[String]] = [
        ["$ npm test", "142 passing, 0 failing", "$ ▌"],
        ["Squashed 4 fixups into 9f30bb7.", "Push the rebased branch?", "▸ waiting for you · 4m"],
        ["$ claude", "Drafting the migration guide…", "(no output for 1h)"],
        ["$ git diff --stat", "Auth/TokenStore.swift  +61 -4", "4 commits ahead of origin"],
        ["$ pytest -x tests/timer", "Isolated the flake to the shared clock.", "$ ▌"],
    ]

    private static let names = [
        "add-retries", "feat/queue-drain", "fix/token-refresh", "chore/deps",
        "spike/watch-sync", "feat/diff-folding", "fix/tmux-resize", "docs/handoff",
        "feat/overview-grid", "fix/first-responder",
    ]

    private static let agents = ["claude", "codex", "shell", "aider"]
}

/// What a pane is in this commit: a name, and nothing behind it.
///
/// It stands in for a terminal and says so. The crossing note beside the title
/// is not a placeholder, though — it is the real behavior: when the pane
/// sliding in belongs to a DIFFERENT workspace, its title carries that
/// workspace's name in muted text, so the crossing is visible while it is
/// still abandonable rather than a surprise you find after committing to it.
struct ShellPanePlaceholder: View {
    let slot: ShellPaneSlot
    /// The canned review pane this slot draws instead of text, under
    /// `-shell-changes`. Nil for every other launch.
    var changes: ChangesStore?

    /// A value that changes if and only if this pane is REBUILT.
    ///
    /// The whole of how the pane-retention invariant is proved, and it works
    /// because of what `@State` is: the initializer runs every time the struct
    /// is created, and SwiftUI keeps the FIRST value for as long as the view's
    /// identity survives. So a `body` pass, a fleet poll, a re-seat of
    /// `position` and a whole swipe all leave this alone; a destroyed and
    /// recreated subtree gets a new one.
    ///
    /// A placeholder can afford to be honest about this in a way a terminal
    /// cannot — a real pane's evidence of being rebuilt is a lost scroll
    /// position, which is not a thing a test can read — so the assertion is
    /// made here, against the same `ShellPaneTrack` the app uses.
    @State private var born = UUID().uuidString.prefix(8)

    private var workspace: ShellWorkspace { slot.workspace }
    private var tab: ShellTab { slot.tab }
    private var isCrossing: Bool { slot.isCrossing }

    /// Whether this launch asked for panes that SCROLL.
    ///
    /// `-shell-scroll`, and it is the only flag in this harness that changes
    /// what a pane IS rather than how many of them there are. A text
    /// placeholder cannot show the one thing a real pane brings with it: its
    /// own vertical gesture, competing with the shell's page turn over the
    /// same touch. A terminal has one and so does a diff, and they are
    /// different recognizers with the same problem — see
    /// `ShellPaneScrollTests`.
    private static var scrolls: Bool {
        CommandLine.arguments.contains("-shell-scroll")
    }

    /// Where the pane's own scroll view is, so a test can tell "the shell
    /// swallowed the scroll" from "there was nothing to scroll".
    @State private var offset: CGFloat = 0

    var body: some View {
        Group {
            if let changes {
                // The real pane, over canned data — not a stand-in for it.
                // What the shell has to arbitrate with is `ChangesView`'s own
                // scroll views, and a fixture that merely looked like a diff
                // would have none of them.
                ChangesView(
                    store: changes, workspaceName: workspace.name, agents: [],
                    pullRequest: nil)
                    // Where the shell's furniture is, told to the pane the same
                    // way `ShellPaneRealView` tells a real one. Without it the
                    // diff's header sits under the clock and a test's drag
                    // lands somewhere the app would never put it.
                    .safeAreaInset(edge: .top, spacing: 0) {
                        Color.clear.frame(height: slot.chrome.top)
                    }
                    .safeAreaInset(edge: .bottom, spacing: 0) {
                        Color.clear.frame(height: slot.chrome.bottom)
                    }
            } else if Self.scrolls {
                scrollingBody
            } else {
                restingBody
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .contentShape(.rect)
        .overlay(alignment: .topLeading) { probe }
    }

    private var restingBody: some View {
        VStack(spacing: PaneMetrics.step) {
            title

            // SF Mono, because everything under here came off a machine — or
            // would have, in the commit that puts a terminal in this slot.
            Text("\(workspace.id) · \(tab.id)")
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(.tertiary)

            Text(slot.isVisible ? "visible" : "hidden")
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    /// The same pane with a `ScrollView` around it, and enough rows that there
    /// is somewhere to go.
    ///
    /// A plain `ScrollView` on purpose, rather than a copy of `ChangesView`.
    /// What the shell has to get right is not a fact about the diff — it is
    /// that a `UIScrollView` inside a pane claims a drag before the shell's
    /// horizontal `DragGesture` can, in any direction, including the one it has
    /// nothing to do with. A pane made of forty lines of text has exactly that
    /// recognizer and nothing else, which is what makes it the right fixture:
    /// a rule proved against it is a rule the third scrollable pane gets for
    /// free.
    private var scrollingBody: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: PaneMetrics.step) {
                title
                ForEach(0..<60, id: \.self) { line in
                    Text("\(tab.id) · line \(line)")
                        .font(.system(size: 12, design: .monospaced))
                        .foregroundStyle(.tertiary)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                Text(slot.isVisible ? "visible" : "hidden")
                    .font(.system(size: 11))
                    .foregroundStyle(.tertiary)
            }
            .padding(PaneMetrics.edge)
        }
        .onScrollGeometryChange(for: CGFloat.self) { $0.contentOffset.y } action: { _, y in
            offset = y
        }
    }

    private var title: some View {
        HStack(spacing: PaneMetrics.step) {
            ShellMarkView(mark: tab.mark, size: 7)
            Text(tab.title)
                .font(.system(size: 17, weight: .medium))
            if isCrossing {
                Text(workspace.name)
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
            }
        }
    }

    /// The two things about a pane that no screenshot can show: whether it
    /// survived the last gesture, and whether it thinks it is the pane.
    ///
    /// A one-point element rather than a value on the pane itself, for the
    /// reason `ShellRootView.probe` gives: `accessibilityValue` on a container
    /// makes the container the element and hides everything inside it.
    ///
    /// `isVisible` is here because it is the flag a real terminal opens its
    /// ssh stream on and the flag a composer takes first responder on, and
    /// `DockedBar.swift:34-41` is what happens when two panes have it at once.
    /// Mid-gesture two panes are on screen, so "exactly one" is a claim that
    /// has to be checked with a finger down.
    private var probe: some View {
        Rectangle()
            .fill(Color.white.opacity(0.001))
            .frame(width: 1, height: 1)
            .accessibilityElement()
            .accessibilityIdentifier("shell-pane-\(tab.id)")
            .accessibilityValue(
                "born=\(born) visible=\(slot.isVisible ? 1 : 0) "
                    + "offset=\(Int(offset.rounded()))")
    }
}

#endif
