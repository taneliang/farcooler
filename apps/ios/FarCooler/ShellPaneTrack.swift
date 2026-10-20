import SwiftUI

// The panes, mounted once and moved rather than rebuilt.
//
// ## The invariant this file is
//
// **A pane must never be rebuilt, and its host's identity must never change.**
// Four comments in this codebase exist because that was got wrong four
// separate ways:
//
// - The pane host, `WorkspaceView`, mounted a pane and never destroyed it,
//   because tearing one down costs a scroll position, a half-typed message and
//   a tmux renegotiation. The screen is gone and the rule is not: it is
//   `ShellRetainedPane` below.
// - A route whose LOOKUP drove the view structure threw every pane away the
//   moment the lookup stopped succeeding — `WorktreeRoute`, which latched the
//   starting pane once for exactly this reason. There are no routes now, and
//   the same trap is here as `ShellPaneRealView`'s `init` latch.
// - `Connection.swift:178-206` — writing focus back into the navigation path
//   changed a path element's value, and SwiftUI is free to rebuild a
//   destination whose value changed. `lastFocus` is still a dictionary beside
//   the navigation rather than inside it, and still for that reason.
// - And the one this file fixes, which the round that built the shell wrote
//   down and left standing: **a three-slot `HStack` keys its children by
//   POSITION.** `ShellRootView` drew `HStack { slot(previous), slot(current),
//   slot(next) }` with each slot's content `.id(tab.id)`, and that is stable
//   only while nothing moves. The moment `position` is re-seated the middle
//   slot's id goes from A to B and the trailing slot's from B to C — so
//   SwiftUI destroys the subtree at slot 1 and builds a fresh one, and the
//   pane for B that was ALREADY MOUNTED one slot over is destroyed too. Every
//   commit rebuilt all three panes. With text placeholders that is invisible;
//   with terminals it is three ssh streams renegotiated per swipe, and the
//   `.carry` release did it in the middle of a flight animation so you would
//   watch it happen.
//
// So identity is decoupled from slot. The panes live in one non-lazy `ZStack`
// keyed by TAB ID, and which of them is on screen is its opacity — a number,
// not a place in a container. Moving `position` changes numbers; nothing is
// destroyed by choosing another tab, and there is no arrangement of the fleet
// that can make one identity land on another's view.
//
// ## Exactly one pane is visible
//
// `DockedBar.swift:34-41` is the sharp edge. An input accessory lives in the
// KEYBOARD's window, so a pane that is merely hidden goes on holding first
// responder and goes on drawing its composer over whatever is on top. Only
// the pane AT REST may be `isVisible`, which is read off `position` alone.

/// One pane's place in the track, as the thing that draws it needs it.
///
/// A struct rather than four positional arguments because the last two are
/// both `Bool` and mean opposite kinds of thing: one is about where the pane
/// is going and one is about what it is allowed to do. A call site that
/// swapped them would compile.
struct ShellPaneSlot {
    var position: ShellPosition
    var worktree: ShellWorktree
    var tab: ShellTab
    /// The room the shell's own furniture takes out of this pane.
    ///
    /// The display's safe area, top and bottom. A pane is laid out FULL BLEED — a worktree fills the
    /// screen, which is the whole point of the shell — so this is not layout,
    /// it is the pane being told where the glass is, exactly the way the pane
    /// host it replaced told its panes where the navigation bar and the tab
    /// strip were. Applied as a `safeAreaInset` rather than as padding, a
    /// scroll view consumes it as a CONTENT inset: rows still travel behind
    /// the bar, they just do not come to rest under it.
    var chrome: EdgeInsets
    /// The pane at rest, and there is exactly one in the whole track.
    ///
    /// What it buys is a stream, a poll and a tmux size assertion — see
    /// `TerminalView` — and what it costs when two panes have it is two
    /// composers fighting over first responder.
    var isVisible: Bool
}

/// A pane the track is keeping alive, and why.
///
/// Keyed on the TAB's id and nothing else. Not on the index and not on the
/// position: an id that moves when the fleet reorders is an id that rebuilds a
/// pane the reorder did not touch.
private struct ShellRetainedPane: Identifiable, Hashable {
    /// The tab this pane is, for the whole of its life.
    let tab: String
    /// The worktree it belongs to, for the scope rule below. By id, because
    /// a worktree index is not stable across a poll.
    let worktree: String

    var id: String { tab }
}

/// The panes of a fleet, mounted and retained.
///
/// This is the pane host's `visited` / `select` / `prune` / `content(of:)`
/// lifted up to the shell. The `.opacity` / `.allowsHitTesting` / `.id(pane.id)`
/// triple is the retention mechanism: hidden and not removed, so the view
/// stays in the hierarchy and its state with it, and unable to swallow a touch
/// meant for the pane on top of it.
struct ShellPaneTrack<Pane: View>: View {
    let fleet: ShellFleet
    /// The pane at rest, and the only source of "which pane is visible".
    let position: ShellPosition
    /// One pane's width, which is the display's.
    let page: CGFloat
    /// The room the display's own furniture takes out of every pane. See
    /// `ShellPaneSlot.chrome`.
    let chrome: EdgeInsets
    /// What a pane is. Handed a slot rather than built here, so this file
    /// never has to know what a pane is.
    let pane: (ShellPaneSlot) -> Pane

    /// Panes that have been at rest at least once, oldest first: an array so
    /// the order is stable, since identity in a `ForEach` is the whole
    /// mechanism here. Only panes actually visited are retained.
    @State private var retained: [ShellRetainedPane] = []

    var body: some View {
        ZStack(alignment: .topLeading) {
            ForEach(mounted) { entry in
                mount(entry)
            }
        }
        // A FIXED width: a flexible frame's lower bound is its child's width,
        // and the shell then became wider than the screen.
        .frame(width: page)
        .frame(maxHeight: .infinity)
        .background { Themes.shared.current.backgroundColor }
        .clipped()
        .contentShape(.rect)
        // The retained set is state, so it is written in a change handler and
        // never in `body`. `mounted` is the union of it and the pane at rest,
        // computed fresh every pass, so a pane is on screen in the frame it is
        // asked for rather than the frame after.
        .onAppear { record() }
        .onChange(of: restingTab) { _, _ in record() }
        .onChange(of: fleetTabIDs) { _, _ in record() }
    }

    /// One pane, mounted, and shown when it's the one at rest.
    @ViewBuilder
    private func mount(_ entry: ShellRetainedPane) -> some View {
        let resting = entry.id == restingTab
        pane(resting ? atRest(entry) : offTrack(entry))
            .frame(width: page)
            .frame(maxHeight: .infinity)
            .background(Themes.shared.current.backgroundColor)
            .clipShape(Rectangle())
            // Hidden, not removed: `opacity` keeps the view in the hierarchy,
            // which is what preserves its state, while `allowsHitTesting`
            // stops a pane nobody can see from swallowing taps.
            .opacity(resting ? 1 : 0)
            .allowsHitTesting(resting)
            // Never recycled onto a different pane.
            .id(entry.id)
    }

    // MARK: - Which panes are mounted

    /// The pane at rest, by id, or nil when the fleet no longer has one there.
    private var restingTab: String? { fleet.tab(at: position)?.id }

    /// Every tab the fleet currently holds, sorted, as something `onChange`
    /// can compare: a worktree gaining a terminal must not read as every pane
    /// moving.
    private var fleetTabIDs: [String] {
        fleet.worktrees.flatMap { $0.tabs.map(\.id) }.sorted()
    }

    private var mounted: [ShellRetainedPane] {
        var out = retained.filter(inScope)
        if let entry = entry(at: position), !out.contains(where: { $0.id == entry.id }) { out.append(entry) }
        return out
    }

    private func entry(at position: ShellPosition) -> ShellRetainedPane? {
        guard fleet.contains(position), let tab = fleet.tab(at: position) else { return nil }
        return ShellRetainedPane(tab: tab.id, worktree: fleet.worktrees[position.worktree].id)
    }

    /// Whether a retained pane is still worth keeping mounted: it still
    /// exists (the pane host's `prune`), and it is in the worktree at rest or
    /// one of its neighbors in the fleet. Never pruned to nothing: a poll
    /// that briefly returns an empty fleet would otherwise unmount every pane.
    private func inScope(_ entry: ShellRetainedPane) -> Bool {
        guard !fleet.isEmpty else { return true }
        guard fleet.position(ofTab: entry.tab) != nil else { return false }
        return nearby.contains(entry.worktree)
    }

    /// The worktree at rest and its two neighbors, by id.
    private var nearby: Set<String> {
        let here = position.worktree
        return Set(
            (here - 1...here + 1)
                .filter { fleet.worktrees.indices.contains($0) }
                .map { fleet.worktrees[$0].id })
    }

    /// Remember the pane at rest, and forget the ones that have gone.
    /// Appended and never inserted, so a pane's place in the `ForEach` only
    /// moves when something before it is dropped.
    private func record() {
        var next = retained.filter(inScope)
        if let entry = entry(at: position), !next.contains(where: { $0.id == entry.id }) {
            next.append(entry)
        }
        guard next != retained else { return }
        retained = next
    }

    // MARK: - What each pane is told

    /// The pane at rest: visible, and the only one.
    private func atRest(_ entry: ShellRetainedPane) -> ShellPaneSlot {
        var slot = offTrack(entry)
        slot.isVisible = true
        return slot
    }

    /// A retained pane: still mounted, and not visible.
    private func offTrack(_ entry: ShellRetainedPane) -> ShellPaneSlot {
        let at = fleet.position(ofTab: entry.tab)
        let worktree = at.flatMap(self.worktree(at:))
        return ShellPaneSlot(
            position: at ?? position,
            worktree: worktree
                ?? ShellWorktree(id: entry.worktree, name: "", tabs: []),
            // Nothing is known about a tab that is not in the fleet any more,
            // and this one is drawn nowhere regardless. `core: nil` declines
            // to state the agent axis rather than claiming it is at a prompt.
            tab: at.flatMap(fleet.tab(at:))
                ?? ShellTab(
                    id: entry.tab, title: "", mark: GlanceMark(attention: .quiet, core: nil)),
            chrome: chrome,
            isVisible: false)
    }

    private func worktree(at position: ShellPosition) -> ShellWorktree? {
        fleet.worktrees.indices.contains(position.worktree)
            ? fleet.worktrees[position.worktree] : nil
    }
}
