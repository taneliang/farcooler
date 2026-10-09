import AgentKit
import Foundation

// Where each workspace was left (ov-442).
//
// `SelectionMemory` keeps where the window is, one place. Switching to another
// workspace used to open that workspace's board, however deep into it you had
// been. This keeps one place per workspace, as `SelectionMemory.encode` writes
// it, so a switch away and back goes back to the task, worktree or plan page
// that was open, and so does a relaunch.

enum WorkspacePlaces {
    typealias Selection = ContentView.Selection

    /// `host|workspace` to `SelectionMemory.encode`'s string, as a dictionary.
    static let key = "nav.workspacePlaces.v1"

    private static func id(host: String, workspace: String) -> String { "\(host)|\(workspace)" }

    /// Keep `selection` as its workspace's place. Anything that isn't inside
    /// a workspace leaves what's kept alone.
    static func remember(_ selection: Selection?, in defaults: UserDefaults = .standard) {
        guard case .workspace(let host, let workspace, _)? = selection,
            let saved = SelectionMemory.encode(selection)
        else { return }
        var kept = defaults.dictionary(forKey: key) as? [String: String] ?? [:]
        let name = id(host: host, workspace: workspace)
        guard kept[name] != saved else { return }
        kept[name] = saved
        defaults.set(kept, forKey: key)
    }

    /// Where a switch from `old` to `new` should open instead of `new`, or
    /// nil to open `new` itself.
    ///
    /// Only a move to a workspace's board, from somewhere that isn't that
    /// workspace, is a switch: opening a task, or stepping up to the board
    /// from inside the workspace, goes where it was sent. A place that's gone
    /// (a worktree removed since) is not returned.
    static func restoring(
        from old: Selection?, to new: Selection?, in fleet: Fleet, defaults: UserDefaults = .standard
    ) -> Selection? {
        guard let old, case .workspace(let host, let workspace, nil)? = new else { return nil }
        if case .workspace(host, workspace, _) = old { return nil }
        let kept = defaults.dictionary(forKey: key) as? [String: String] ?? [:]
        guard let saved = kept[id(host: host, workspace: workspace)],
            let place = SelectionMemory.decode(saved),
            case .workspace(host, workspace, let focus?) = place
        else { return nil }
        if case .worktree(let worktree, _) = focus,
            WorkspaceSelection.worktree(host: host, id: worktree, in: fleet) == nil
        {
            return nil
        }
        return place
    }
}

/// The window's side of a switch: `ContentView` feeds it each change of
/// selection and does what it says, so the sequence (the board a switch
/// lands on, the place opened in its stead, the step history records) is
/// pinned without a window.
struct PlaceSwitch {
    typealias Selection = ContentView.Selection

    /// What a change of selection asks of the window.
    enum Step: Equatable {
        /// Open this instead, and do nothing else for the change just made.
        case open(Selection)
        /// Carry on with the change as a step from `from`, which is where
        /// a switch began when this change was the place opened for one.
        case carryOn(from: Selection?)
    }

    private var origin: Selection??

    mutating func changed(
        from old: Selection?, to new: Selection?, in fleet: Fleet, defaults: UserDefaults = .standard
    ) -> Step {
        if origin == nil, let place = WorkspacePlaces.restoring(from: old, to: new, in: fleet, defaults: defaults) {
            origin = .some(old)
            return .open(place)
        }
        let from = origin ?? old
        origin = nil
        WorkspacePlaces.remember(new, in: defaults)
        return .carryOn(from: from)
    }
}
