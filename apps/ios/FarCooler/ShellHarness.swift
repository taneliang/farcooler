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
                openingOnOverview: CommandLine.arguments.contains("-shell-overview")
            ) {
                _, workspace, tab, isCrossing in
                ShellPanePlaceholder(workspace: workspace, tab: tab, isCrossing: isCrossing)
            }
        }
        .preferredColorScheme(.dark)
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
                // a one-tab workspace in the middle because
                // `columnSelection`'s cap and the overview's reach both depend
                // on the count and both have to be reachable by flag alone.
                let tabs = [3, 2, 5, 1, 4][index % 5]
                return ShellWorkspace(
                    id: "ws-\(index)",
                    name: Self.names[index % Self.names.count]
                        + (index >= Self.names.count ? "-\(index / Self.names.count + 1)" : ""),
                    // A server on some of them and not others, so the bar's
                    // both-ways case is reachable by flag: the local runner
                    // says nothing, the rest name themselves.
                    server: index % 3 == 0 ? nil : Self.servers[index % Self.servers.count],
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
                            mark: mark(workspace: index, tab: tab))
                    })
            })
    }

    /// The four states, spread so that every one of them is on screen at the
    /// scales a person looks at.
    private static func mark(workspace: Int, tab: Int) -> ShellMark {
        // Tab 0 is the diff now, so "the first AGENT" is tab 1. This read
        // `tab == 0` while the fixture put the diff last, and moving the diff
        // to the front left that arm unreachable — every workspace would have
        // quietly rendered as working, and the amber mark the whole ribbon
        // exists for would have been absent from every screenshot.
        switch (workspace % 4, tab == 0) {
        case (0, true): return .unreadDiff
        case (1, false) where tab == 1: return .needsYou
        case (2, _): return .stale
        default: return .working
        }
    }

    private static let servers = ["gpu-box-2", "eu-runner-1"]

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
    let workspace: ShellWorkspace
    let tab: ShellTab
    let isCrossing: Bool

    var body: some View {
        VStack(spacing: PaneMetrics.step) {
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

            // SF Mono, because everything under here came off a machine — or
            // would have, in the commit that puts a terminal in this slot.
            Text("\(workspace.id) · \(tab.id)")
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(.tertiary)

            Text("Pane placeholder")
                .font(.system(size: 11))
                .foregroundStyle(.tertiary)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .contentShape(.rect)
    }
}

#endif
