import SwiftUI

#if DEBUG

// The navigation shell over a canned worktree, so it can be driven with no
// runner behind it.
//
// `-shell-harness`, alongside `-agent-layout-harness` and
// `-changes-layout-harness` in `FarCoolerApp.swift`, and for the same reason
// those two exist: a surface that has only ever been argued about from the
// code is a surface nobody has looked at. The shell is a GESTURE, and a
// gesture cannot be reviewed in a screenshot at all. It has to be swiped, and
// swiping it needs a worktree with tabs, and a worktree needs a runner, a
// daemon and some agents actually doing something.
//
// It is the shell as the phone mounts it (`ShellScreen`, scoped to one
// worktree): one worktree, three tabs, the bar and its column. It used to
// stand on a whole canned fleet, with an overview to lift into and a
// neighbor worktree to cross to, and the app has no such shell any more —
// see `ShellScope`. What is canned is only what a runner would supply: the
// panes, which are placeholders that say so, or a scrolling pane under
// `-shell-scroll` and the review pane over a canned diff under
// `-shell-changes`.
//
// `ShellGestureTests`, `ShellColumnCloseTests` and `ShellPaneScrollTests`
// drive this harness, which is the only way the axis lock, the commit
// threshold and the page turn over a scrolling pane get a test at all.

/// The shell, standing on a fixture.
struct ShellHarness: View {
    static var isRequested: Bool {
        CommandLine.arguments.contains("-shell-harness")
    }

    /// The one thing a canned `ChangesView` needs, and the same one
    /// `ChangesLayoutHarness` stands it on: a connection nobody connects, for
    /// the sake of the store hanging off it.
    @StateObject private var connection = Connection()

    /// The canned worktree, held rather than rebuilt, so a close has something
    /// to change: `onCloseTab` is what a runner does with the request, applied
    /// here in its place.
    @State private var fleet = Self.fleet

    @State private var request: String?

    var body: some View {
        ZStack {
            // A ground for the glass to be glass against. The panes are text
            // on nothing, and glass over nothing has no material to sample —
            // the bar would read as a gray rectangle and every screenshot of
            // it would be a screenshot of the wrong thing.
            LinearGradient(
                colors: [Color(red: 0.06, green: 0.07, blue: 0.09), Color(red: 0.02, green: 0.02, blue: 0.03)],
                startPoint: .top, endPoint: .bottom)
                .ignoresSafeArea()

            ShellRootView(
                fleet: fleet,
                initial: ShellPosition(worktree: 0, tab: 0),
                request: $request,
                // A close, done the way a runner does it: the tab leaves the
                // fleet a round trip later and the shell's vanish rule takes
                // it from there. Only so `ShellColumnCloseTests` can see that a
                // Close tap reached this closure at all — the confirmation and
                // the two calls are `ShellScreen`'s, and a fixture has no
                // runner for them.
                //
                // **Not inside the swipe action.** Removing the row from the
                // data source while the row's own contextual action is still
                // running asks the list for a batch update in the middle of
                // one, and UIKit aborts the process ("invalid number of items
                // in section"). A real runner never answers synchronously, so
                // neither does this.
                onCloseTab: { worktree, tab in
                    Task { @MainActor in
                        try? await Task.sleep(for: .milliseconds(300))
                        guard
                            let index = fleet.worktrees.firstIndex(where: { $0.id == worktree.id })
                        else { return }
                        fleet.worktrees[index].tabs.removeAll { $0.id == tab.id }
                    }
                }
            ) { slot in
                ShellPanePlaceholder(slot: slot, changes: changesStore)
            }
        }
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

    /// The canned worktree: `add-retries`, with three tabs.
    ///
    /// Tab 0 is Changes, as it is in every worktree the app shows, and only
    /// a Changes tab is ever `unreadDiff`, which is a model rule rather than a
    /// style choice. The other two are terminals and can be closed; Changes
    /// cannot, and a fixture that marked all three alike would let
    /// `ShellColumnCloseTests` pass against a column that offered Close on it.
    static var fleet: ShellFleet {
        ShellFleet(
            worktrees: [
                ShellWorktree(
                    id: "ws-0",
                    name: "add-retries",
                    tabs: (0..<3).map { tab in
                        ShellTab(
                            id: "ws-0-tab-\(tab)",
                            title: tab == 0 ? "Changes" : Self.agents[tab % Self.agents.count],
                            mark: mark(tab: tab).mark,
                            wantsAttention: mark(tab: tab).wantsAttention,
                            closable: tab != 0)
                    })
            ])
    }

    /// The marks the bar can draw, one per tab: the Changes tab with something
    /// to review (which never states an agent core), an agent at a prompt for
    /// a person, and an agent producing.
    static func mark(tab: Int) -> (mark: GlanceMark, wantsAttention: Bool) {
        switch tab {
        case 0: return (GlanceMark(attention: .toReview, core: nil), false)
        case 1: return (GlanceMark(attention: .needsYou, core: .atAPrompt), true)
        default: return (GlanceMark(attention: .quiet, core: .producing), false)
        }
    }

    private static let agents = ["claude", "codex", "shell", "aider"]
}

/// What a pane is here: a name, and nothing behind it.
///
/// It stands in for a terminal and says so.
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

    private var worktree: ShellWorktree { slot.worktree }
    private var tab: ShellTab { slot.tab }

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
                    store: changes, worktreeName: worktree.name, agents: [],
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
            Text("\(worktree.id) · \(tab.id)")
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
