import SwiftUI

/// What the focused diff can do, for the Diff menu (ov-211).
///
/// The Diff menu's items were enabled whatever the window showed, so with a
/// terminal focused and no diff anywhere, Next Hunk, Read Commit by Commit
/// and Mark as Reviewed all looked ready and did nothing. The focused
/// `ChangesPane` now publishes this, and only it: with no diff focused it's
/// nil and every item is dimmed. HIG, The menu bar: "If a menu bar item isn't
/// actionable, disable the action instead of hiding it from the menu."
struct DiffMenuFocus: Equatable {
    var nextHunk = false
    var previousHunk = false
    var nextFile = false
    var previousFile = false
    /// The branch has commits to read one at a time.
    var readsCommits = false
    /// Reading commit by commit, with one further along in that direction.
    var nextCommit = false
    var previousCommit = false
    /// The worktree has changed since it was last marked reviewed.
    var marksReviewed = false

    /// Each answer is the one the pane's own move would give: a move is
    /// enabled exactly when `DiffWalk.step` wouldn't `.stay`.
    ///
    /// `unreviewed` is the fleet inbox's word for this worktree, or nil
    /// before the inbox has one; then a diff with files in it is taken as
    /// unreviewed, which is what the daemon assumes of a worktree never
    /// marked.
    static func make(
        scope: DiffScope, files: Int, at current: Int?, hunks: [String], lastHunk: String?,
        next: ChangeCommit?, previous: ChangeCommit?, commits: Int, unreviewed: Bool?
    ) -> DiffMenuFocus {
        func moves(_ direction: Int, hunks: [String]) -> Bool {
            let boundary = DiffWalk.boundary(direction, scope: scope, next: next, previous: previous)
            return DiffWalk.step(
                direction, hunks: hunks, after: lastHunk, files: files, at: current, boundary: boundary) != .stay
        }
        return DiffMenuFocus(
            nextHunk: moves(1, hunks: hunks), previousHunk: moves(-1, hunks: hunks),
            nextFile: moves(1, hunks: []), previousFile: moves(-1, hunks: []),
            readsCommits: commits > 0,
            nextCommit: scope == .commit && next != nil,
            previousCommit: scope == .commit && previous != nil,
            marksReviewed: unreviewed ?? (files > 0))
    }

    /// A Diff menu item acts only with a diff focused in the key main window,
    /// nothing over it, and only when `can` says this diff can.
    static func allows(
        _ can: KeyPath<DiffMenuFocus, Bool>, _ diff: DiffMenuFocus?, in window: MainWindowFocus?
    ) -> Bool {
        guard MainWindowFocus.navigates(window), let diff else { return false }
        return diff[keyPath: can]
    }
}

/// What the Layout menu can do with the worktree on screen (ov-211).
struct LayoutMenuFocus: Equatable {
    /// The layout has more than one pane: zoom, even out, arrange, step
    /// between panes, move one out.
    var panes = 0
    /// The key pane is zoomed: Zoom Pane carries a checkmark.
    var zoomed = false
    /// The key pane has a neighbor on that side.
    var neighbors: Set<TileDirection> = []
    /// Next Layout and Previous Layout go somewhere.
    var stepsLayouts = false
    /// The key pane is an agent that can show its chat or its terminal.
    var switchesMode = false
    /// The key pane is Claude in a terminal, and its conversation view is
    /// offered (`NativeAgents.offers`).
    var switchesConversation = false
    /// Why it isn't, for the item's help (ov-443): nil where it is, or where
    /// there's no pane.
    var conversationUnavailable: String?
    /// The runner serves web panes (`web_pane`): against an older one, Open
    /// Web Page would take an address and then only fail (M5, ov-435 review
    /// 1). HIG: dim an item that can't act.
    var opensWebPage = false

    /// Splitting and New Layout need only a worktree.
    var splits: Bool { true }
    var zooms: Bool { panes > 1 }
    var arranges: Bool { panes > 1 }
    var stepsPanes: Bool { panes > 1 }
    var movesOut: Bool { panes > 1 }
    var left: Bool { neighbors.contains(.left) }
    var right: Bool { neighbors.contains(.right) }
    var above: Bool { neighbors.contains(.top) }
    var below: Bool { neighbors.contains(.bottom) }

    /// `here` is the pane a keystroke acts on, as `ContentView.tile(_:)`
    /// finds it; `layouts` the ones the bar offers.
    static func make(
        group: PaneGroup?, here: PaneRect?, layouts: [PaneGroup], switchesMode: Bool,
        switchesConversation: Bool = false, conversationUnavailable: String? = nil, opensWebPage: Bool = false
    ) -> LayoutMenuFocus {
        let neighbors = Set(TileDirection.allCases.filter { side in
            guard let group, let here else { return false }
            return group.neighbour(of: here.id, side) != nil
        })
        return LayoutMenuFocus(
            panes: group?.panes.count ?? 0, zoomed: group?.zoomed != nil, neighbors: neighbors,
            // `ContentView.layout(stepping:from:in:)`'s rule: somewhere else
            // to go only with another layout, from one the bar lists.
            stepsLayouts: layouts.count > 1 && layouts.contains { $0.id == group?.id },
            switchesMode: switchesMode, switchesConversation: switchesConversation,
            conversationUnavailable: switchesConversation ? nil : conversationUnavailable, opensWebPage: opensWebPage)
    }
}

/// Publishes the focused diff's `DiffMenuFocus` to the menu bar.
///
/// A view of its own so it can watch the client's inbox, which says whether
/// there's anything to mark reviewed, without the whole pane redrawing on it.
/// Mounted by `ChangesPane` only while it's the focused pane, so one diff in
/// a window speaks for the Diff menu at a time.
struct DiffMenuPublisher: View {
    @ObservedObject var changes: ChangesStore
    @ObservedObject var client: DaemonClient
    let hunks: [String]
    let lastHunk: String?

    var body: some View {
        let order = changes.reviewOrder
        Color.clear.focusedSceneValue(
            \.diffMenu,
            DiffMenuFocus.make(
                scope: changes.scope, files: order.count,
                at: order.firstIndex { $0.path == changes.selectedFile },
                hunks: hunks, lastHunk: lastHunk,
                next: changes.nextCommit, previous: changes.previousCommit,
                commits: changes.commitsInOrder.count,
                unreviewed: client.changesInbox[changes.worktree.short]?.changedSinceReviewed))
    }
}

extension FocusedValues {
    @Entry var diffMenu: DiffMenuFocus?
}
