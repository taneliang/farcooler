import Foundation

// Where a stack of screens goes on an iPhone once the Plan is a sheet
// (ov-616). A workspace screen is its conversation; the Plan opens over it as
// a sheet with a navigation stack of its own, and the screens the Board and
// Themes segments used to push (a task, a tree level, a plan page, a board's
// history) are pushed inside that sheet. A worktree still covers everything.
//
// A stack is still kept and linked to as one list of routes
// (`PhoneLaunch.encode`, `PhoneDestination.link`): this splits that list into
// what goes where, and joins it back for keeping.

extension PhoneRoute {
    /// Whether the screen opens inside the Plan sheet rather than on the stack.
    var opensInPlan: Bool {
        switch self {
        case .task, .history, .plan, .tree: true
        case .workspace, .worktree, .agents: false
        }
    }

    /// The workspace a screen is in, when it is in one.
    var workspacePlace: PhoneWorkspace? {
        switch self {
        case .workspace(let place), .task(let place, _), .history(let place, _), .plan(let place, _), .tree(let place, _):
            place
        case .worktree, .agents:
            nil
        }
    }
}

/// The Plan sheet over a workspace, and the screens pushed inside it.
struct PhonePlanSheet: Identifiable, Hashable, Sendable {
    var place: PhoneWorkspace
    var path: [PhoneRoute] = []

    var id: String { place.stored }
}

/// A stack, laid out: the screens pushed on the phone's stack, the Plan sheet
/// over the top one, and the worktree covering both. At most one of the
/// sheet and the cover.
struct PhoneStackLayout: Equatable, Sendable {
    var path: [PhoneRoute]
    var sheet: PhonePlanSheet?
    var cover: PhoneRoute?

    /// Lay out `stack`. `sheets` says whether a workspace's Plan is a sheet:
    /// false for one whose screen is the Plan itself (no orchestrator), and
    /// on an iPad, whose screens are pushed as before.
    ///
    /// A worktree on top covers the stack, and the Plan's screens under it
    /// go: Back from a worktree is the workspace it was opened from, so the
    /// list is two steps away (ov-617). Otherwise the screens after the last
    /// workspace whose Plan is a sheet open in that sheet.
    static func of(_ stack: [PhoneRoute], sheets: (PhoneWorkspace) -> Bool) -> PhoneStackLayout {
        var routes = stack
        var cover: PhoneRoute?
        if let last = routes.last, case .worktree = last {
            cover = last
            routes.removeLast()
        }
        guard let at = routes.lastIndex(where: { route in
            if case .workspace(let place) = route { return sheets(place) }
            return false
        }), case .workspace(let place) = routes[at] else {
            return PhoneStackLayout(path: routes, sheet: nil, cover: cover)
        }
        let after = routes[(at + 1)...]
        let inside = after.filter { $0.opensInPlan && $0.workspacePlace == place }
        let path = Array(routes[...at]) + after.filter { !($0.opensInPlan && $0.workspacePlace == place) }
        if cover != nil || inside.isEmpty {
            return PhoneStackLayout(path: path, sheet: nil, cover: cover)
        }
        return PhoneStackLayout(path: path, sheet: PhonePlanSheet(place: place, path: inside), cover: nil)
    }

    /// The stack again, as it's kept: the pushed screens, the sheet's, then
    /// the worktree.
    var stack: [PhoneRoute] { path + (sheet?.path ?? []) + (cover.map { [$0] } ?? []) }
}

/// Whether a screen of the phone's stack is the one on top (ov-791): what a
/// screen under a pushed one asks before building its tree again on a board
/// move, which no one would see, and which dropped a frame each in the
/// screen on top while the board moved.
///
/// `own` is the screen's route, or nil for a Plan sheet's own root, which
/// is on top while nothing is pushed in the sheet and nothing covers it.
/// `stack` is `PhoneNavigator.stack`: the pushed screens, the sheet's, the
/// worktree; `covered` is a worktree or the agents view over all of it.
enum ScreenCover {
    static func onTop(_ own: PhoneRoute?, stack: [PhoneRoute], sheet: PhonePlanSheet?, covered: Bool) -> Bool {
        if let own { return stack.last == own }
        guard let sheet, !covered else { return false }
        return sheet.path.isEmpty
    }
}
