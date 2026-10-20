import SwiftUI

// A worktree, full screen: its panes, one at a time, under each pane's own
// navigation bar (`ShellPaneBar.swift`).
//
// It was the navigation shell: a bar at the bottom that WAS the worktree,
// swiped sideways for the next one and lifted for a column of its tabs, with
// the page carried under the finger. The phone's own stack took over the
// navigation (spec §6.1), and the bar's swipe-up came to fight it: a worktree
// is a cover over the stack, Back on the pane's bar takes it away, and
// nothing else needs a gesture of its own (ov-617). A worktree's tabs and
// Close Terminal are in the pane bar's More menu.
//
// What is left here is the part that was never about the finger:
//
// - **A pane must never be rebuilt, and its host's identity must never
//   change.** `ShellPaneTrack` owns which panes are mounted and which single
//   one is `isVisible`; read its header before changing anything that moves
//   `position`.
// - **The fleet moves under the screen.** Every poll rebuilds `fleet` whole,
//   so `position` is re-seated onto whatever index the tab it is anchored to
//   occupies now (`ShellFleet.reseat`), silently.
// - **One arrival, announced once** (`onRest`), and the one way in from
//   outside, a one-shot `request` for a tab by id.

/// A worktree's tabs as its panes' bars offer them: which one is up, the way
/// to another, and Close.
struct ShellTabsChoice {
    var tabs: [ShellTab]
    var current: String?
    var choose: (String) -> Void
    var close: (ShellTab) -> Void
}

extension EnvironmentValues {
    /// The worktree's tabs, for a pane's bar to switch between and close.
    /// Nil outside a worktree.
    @Entry var shellTabs: ShellTabsChoice? = nil

    /// The display's own bottom inset — the home indicator — as the shell
    /// measured it.
    ///
    /// Read by the one kind of pane that needs it: one whose content is inside
    /// a `NavigationStack`, which is a `UINavigationController` and re-derives
    /// this same inset from the window on its own. Everything else wants
    /// `ShellPaneSlot.chrome`, which already includes it.
    ///
    /// Zero outside the shell, which is the honest answer for a pane mounted
    /// somewhere with no shell furniture over it.
    @Entry var shellDisplayBottom: CGFloat = 0
}

/// A worktree's panes, with panes the caller supplies.
///
/// Generic over the pane so this file never has to know what a pane is:
/// `ShellScreen` hands it a terminal, an agent or a diff.
struct ShellRootView<Pane: View>: View {
    let fleet: ShellFleet
    /// A tab chosen to close, from a pane's More menu. The shell chooses
    /// nothing afterwards, on purpose: `ShellFleet.reseat(_:holding:)` runs on
    /// every poll and already answers which tab is up once the anchored one is
    /// gone. Resolving the tab into a terminal is `ShellScreen`'s.
    private let onCloseTab: (ShellWorktree, ShellTab) -> Void
    private let pane: (ShellPaneSlot) -> Pane
    /// A tab to go to, by id, honored once and cleared: a tapped card's link,
    /// a terminal just made, or a tab chosen from a pane's More menu.
    ///
    /// A request rather than a binding to `position`, so nothing outside can
    /// write into the value the fleet's re-seats keep honest.
    @Binding var request: String?
    /// Called when the pane AT REST changes, and at no other time.
    /// `ShellScreen` is the single writer of `Notifier.shared.visibleTerminal`,
    /// and this is the event it writes from.
    private let onRest: ((ShellPosition, ShellArrival) -> Void)?

    /// Where the shell is. Kept honest by `anchor`: a pair of indices into a
    /// fleet that is re-merged from every runner on every poll.
    @State private var position: ShellPosition

    /// The tab `position` is ON, by id, and the thing `onRest` announces.
    /// See `ShellFleet.reseat(_:holding:)`, in AgentKit, for the rule.
    @State private var anchor: String?

    /// Which mount of this view the probe reports, so a UI test can tell the
    /// same shell, moved, from a new one standing in the same place.
    @State private var mount = String(UUID().uuidString.prefix(8))

    init(
        fleet: ShellFleet,
        initial: ShellPosition,
        request: Binding<String?> = .constant(nil),
        onRest: ((ShellPosition, ShellArrival) -> Void)? = nil,
        onCloseTab: @escaping (ShellWorktree, ShellTab) -> Void = { _, _ in },
        @ViewBuilder pane: @escaping (ShellPaneSlot) -> Pane
    ) {
        self.fleet = fleet
        self.onCloseTab = onCloseTab
        self.pane = pane
        _request = request
        self.onRest = onRest
        _position = State(initialValue: initial)
    }

    var body: some View {
        // Two readers: the outer is asked where the safe area is, the inner
        // how big the screen is, and each is measured somewhere it can still
        // tell the truth, since the panes are laid out full bleed.
        GeometryReader { safe in
            let insets = safe.safeAreaInsets
            GeometryReader { full in
                track(page: full.size.width, safeArea: insets)
            }
            .ignoresSafeArea()
        }
        // The keyboard is not furniture: `TerminalView` takes its room itself,
        // and a composer rises with it in the keyboard's own window.
        .ignoresSafeArea(.keyboard, edges: .bottom)
    }

    private func track(page: CGFloat, safeArea: EdgeInsets) -> some View {
        ShellPaneTrack(
            fleet: fleet, position: position, page: page,
            chrome: EdgeInsets(top: safeArea.top, leading: 0, bottom: safeArea.bottom, trailing: 0),
            pane: pane
        )
        .environment(\.shellDisplayBottom, safeArea.bottom)
        .environment(\.shellTabs, tabsChoice)
        .overlay(alignment: .topLeading) { probe }
        // One announcement per arrival. On appear as well as on change: the
        // first pane is one nobody moved to and is still the pane being read,
        // and a cold launch's link is already set when this mounts.
        .onAppear {
            settle(position, in: fleet, as: .appeared)
            honorRequest()
        }
        .onChange(of: position) { _, at in settle(at, in: fleet, as: .moved) }
        // The fleet moved under the shell: re-seated silently onto the tab it
        // is anchored to. `after:` the fleet it replaces, for a fleet that
        // arrives after an empty one.
        .onChange(of: fleet) { old, now in
            let seated = now.reseat(position, holding: anchor, after: old)
            if seated != position {
                var silent = Transaction()
                silent.disablesAnimations = true
                withTransaction(silent) { position = seated }
            }
            settle(seated, in: now, as: .reseated)
        }
        .onChange(of: request) { _, _ in honorRequest() }
    }

    /// The worktree's tabs, for its panes' bars.
    private var tabsChoice: ShellTabsChoice? {
        guard let worktree = currentWorktree else { return nil }
        return ShellTabsChoice(
            tabs: worktree.tabs, current: fleet.tab(at: position)?.id,
            choose: { request = $0 },
            close: { tab in onCloseTab(worktree, tab) })
    }

    /// A pane has come to rest, if a DIFFERENT one has: asked of the tab and
    /// not the index, since a re-merge moves the index without moving the
    /// tab, and another runner's worktree can slide under an index that
    /// holds still.
    private func settle(_ at: ShellPosition, in fleet: ShellFleet, as arrival: ShellArrival) {
        let arrived = fleet.tab(at: at)?.id
        guard arrived != anchor else { return }
        anchor = arrived
        onRest?(at, arrival)
    }

    /// Go where something asked to go, once. Cleared before anything else
    /// can return, so a request a guard turned down never blocks the next one
    /// naming the same pane. Silent: nothing on screen moved toward it.
    private func honorRequest() {
        guard let id = request else { return }
        request = nil
        guard let target = fleet.position(ofTab: id), target != position else { return }
        var silent = Transaction()
        silent.disablesAnimations = true
        withTransaction(silent) { position = target }
    }

    private var currentWorktree: ShellWorktree? {
        fleet.worktrees.indices.contains(position.worktree) ? fleet.worktrees[position.worktree] : nil
    }

    /// The one way a UI test can ask this shell where it is: a one-point
    /// element in the corner, since `accessibilityValue` on a container would
    /// hide everything inside it. `busy` is always 0 now, kept for the
    /// readers that wait on it (`ShellState.swift`).
    private var probe: some View {
        Rectangle()
            .fill(Color.white.opacity(0.001))  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
            .frame(width: 1, height: 1)  // style-exempt: DEBUG probe: a near-invisible hit target the UI tests read, not a fill
            .accessibilityElement()
            .accessibilityIdentifier("shell-state")
            .accessibilityValue(
                "ws=\(position.worktree) tab=\(position.tab) worktrees=\(fleet.worktrees.count) "
                    + "tabs=\(fleet.tabCount(ofWorktree: position.worktree)) mount=\(mount) busy=0")
    }
}
