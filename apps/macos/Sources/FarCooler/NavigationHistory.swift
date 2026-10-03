import Foundation

/// Back and forward through where the window has been (ov-192): ⌃⌘← and
/// ⌃⌘→, Xcode's pair, since ⌘[ and ⌘] walk the terminals.
///
/// Places, not panes: choosing another pane in the worktree you're in is no
/// step (`WorkspaceSelection.place`). A worktree opened from its task keeps
/// that task with it (`Stop.trail`), so coming back to it brings its task's
/// crumb and menu back too (ov-185). A step that lands where it was asked to
/// isn't recorded as a new one, and a place that's gone since (a worktree
/// removed, a workspace deleted) is passed over, not landed on.
struct NavigationHistory: Equatable {
    typealias Selection = ContentView.Selection

    /// One place been to, with the task it was opened from, if any.
    struct Stop: Equatable {
        var place: Selection
        var trail: Selection?

        init(_ place: Selection, trail: Selection? = nil) {
            self.place = WorkspaceSelection.place(place)
            self.trail = trail
        }
    }

    /// How far back it goes.
    static let limit = 50

    private(set) var back: [Stop] = []
    private(set) var forward: [Stop] = []
    /// Where a Back or Forward is going: its own change of selection isn't a
    /// new step.
    private var stepping: Selection?

    var canGoBack: Bool { !back.isEmpty }
    var canGoForward: Bool { !forward.isEmpty }

    /// The window's selection went from `old`, opened from `trail`, to
    /// `new`.
    mutating func record(from old: Selection?, to new: Selection?, trail: Selection? = nil) {
        if let stepping, WorkspaceSelection.samePlace(stepping, new) {
            self.stepping = nil
            return
        }
        stepping = nil
        guard let old, new != nil, !WorkspaceSelection.samePlace(old, new) else { return }
        let stop = Stop(old, trail: trail)
        if back.last != stop { back.append(stop) }
        if back.count > Self.limit { back.removeFirst(back.count - Self.limit) }
        forward = []
    }

    /// ⌃⌘←: the last place before `current` that still `resolves`, or nil
    /// with none. `current`, with its `trail`, goes onto Forward.
    mutating func goBack(from current: Selection?, trail: Selection? = nil, resolves: (Selection) -> Bool) -> Stop? {
        step(from: current, trail: trail, take: \.back, give: \.forward, resolves: resolves)
    }

    /// ⌃⌘→: the place Back left, or nil with none.
    mutating func goForward(from current: Selection?, trail: Selection? = nil, resolves: (Selection) -> Bool) -> Stop? {
        step(from: current, trail: trail, take: \.forward, give: \.back, resolves: resolves)
    }

    private mutating func step(
        from current: Selection?, trail: Selection?, take: WritableKeyPath<Self, [Stop]>,
        give: WritableKeyPath<Self, [Stop]>, resolves: (Selection) -> Bool
    ) -> Stop? {
        while let next = self[keyPath: take].popLast() {
            guard resolves(next.place), !WorkspaceSelection.samePlace(next.place, current) else { continue }
            if let current { self[keyPath: give].append(Stop(current, trail: trail)) }
            stepping = next.place
            return next
        }
        return nil
    }

    /// What ⌃⌘← does.
    enum BackRoute: Equatable {
        /// Back to where you were.
        case history(Stop)
        /// Today's Back (`WorkspaceNavigation.backStep`): leave Focus first,
        /// else up a level, for a window with nowhere to go back to.
        case upALevel
    }

    /// ⌃⌘←: in Focus, leaving it, as before; else where you were; else up a
    /// level.
    mutating func back(focus: Bool, from current: Selection?, trail: Selection?, resolves: (Selection) -> Bool)
        -> BackRoute
    {
        guard !focus, let stop = goBack(from: current, trail: trail, resolves: resolves) else { return .upALevel }
        return .history(stop)
    }
}

extension NavigationHistory {
    /// Whether `place` is still somewhere to go in `fleet`: its workspace
    /// listed, its worktree there. A task's own presence is the board's,
    /// which the window checks when it opens one.
    static func resolves(_ place: Selection, in fleet: Fleet, repositories: (String) -> [String]) -> Bool {
        switch place {
        case .needsYou:
            return true
        case .looseWorktree(let host, let id, _):
            return WorkspaceSelection.worktree(host: host, id: id, in: fleet) != nil
        case .workspace(let host, let id, let focus):
            guard WorkspaceScreen.workspace(id, host: host, in: fleet, repositories: repositories(host)) != nil else {
                return false
            }
            if case .worktree(let worktree, _)? = focus {
                return WorkspaceSelection.worktree(host: host, id: worktree, in: fleet) != nil
            }
            return true
        }
    }
}
