import AgentKit
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
    /// Where a row of the history list sits: a step away from the place the
    /// window is at. The distance counts every stop on that side, 0 for the
    /// nearest, including any a list leaves out as gone, since that is what
    /// `go(toBack:)` and `go(toForward:)` take.
    enum Spot: Equatable {
        case forward(Int)
        case current
        case back(Int)
    }

    struct Row: Equatable {
        var spot: Spot
        var stop: Stop
    }

    /// How many places a side of the list shows.
    static let rowsPerSide = 15

    /// The list a long press on Back or Forward shows, one for both buttons:
    /// Forward's stops, farthest first, then the place the window is at, then
    /// Back's, nearest first, up to `rowsPerSide` each side. A stop that's
    /// known to be gone (`resolves`) is left out.
    func rows(current: Selection?, trail: Selection?, resolves: (Selection) -> Bool) -> [Row] {
        func side(_ stops: [Stop], _ spot: (Int) -> Spot) -> [Row] {
            let near = stops.reversed().enumerated().filter { resolves($0.element.place) }.prefix(Self.rowsPerSide)
            return near.map { Row(spot: spot($0.offset), stop: $0.element) }
        }
        var rows = side(forward) { .forward($0) }.reversed() as [Row]
        if let current { rows.append(Row(spot: .current, stop: Stop(current, trail: trail))) }
        return rows + side(back) { .back($0) }
    }

    /// A row of the list chosen, whichever side it's on. Nil for the place
    /// the window is at, and for one that isn't there.
    mutating func go(to spot: Spot, from current: Selection?, trail: Selection?) -> Stop? {
        switch spot {
        case .back(let distance): go(toBack: distance, from: current, trail: trail)
        case .forward(let distance): go(toForward: distance, from: current, trail: trail)
        case .current: nil
        }
    }

    /// A row of Back's side chosen: the stop `distance` away (0 the nearest)
    /// in one move. The stops passed over, and `current`, go onto Forward in
    /// the order they'd be walked back through, as Safari does. Nil, and
    /// nothing moved, for a distance there's nothing at.
    mutating func go(toBack distance: Int, from current: Selection?, trail: Selection?) -> Stop? {
        jump(distance, from: current, trail: trail, take: \.back, give: \.forward)
    }

    /// `go(toBack:from:trail:)`, the other way.
    mutating func go(toForward distance: Int, from current: Selection?, trail: Selection?) -> Stop? {
        jump(distance, from: current, trail: trail, take: \.forward, give: \.back)
    }

    private mutating func jump(
        _ distance: Int, from current: Selection?, trail: Selection?, take: WritableKeyPath<Self, [Stop]>,
        give: WritableKeyPath<Self, [Stop]>
    ) -> Stop? {
        guard distance >= 0, distance < self[keyPath: take].count else { return nil }
        let passed = Array(self[keyPath: take].suffix(distance + 1).reversed())
        self[keyPath: take].removeLast(distance + 1)
        if let current { self[keyPath: give].append(Stop(current, trail: trail)) }
        self[keyPath: give].append(contentsOf: passed.dropLast())
        stepping = passed.last?.place
        return passed.last
    }

    /// Whether `place` is still somewhere to go in `fleet`: its workspace
    /// listed, its worktree there, its task on its board once that has been
    /// read (`board` gives a workspace's, by runner and id, or nil when the
    /// window hasn't one; a board with no rows isn't read yet, and says
    /// nothing is gone).
    static func resolves(
        _ place: Selection, in fleet: Fleet, repositories: (String) -> [String],
        board: (_ host: String, _ workspace: String) -> TaskBoardModel? = { _, _ in nil }
    ) -> Bool {
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
            if case .task(let task)? = focus, let rows = board(host, id)?.columns.flatMap(\.rows), !rows.isEmpty {
                return rows.contains { $0.id == task }
            }
            return true
        }
    }
}
