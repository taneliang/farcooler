import AgentKit
import Foundation

// Where the window reopens (spec §4.6 and §9).
//
// Where the window was is kept as a `Destination`, in `nav.destination.v1`
// (ov-182): the workspace and what's open in it, a task's tab and agent, the
// pane the keyboard was in. It replaces `workspace.lastSelection`, the
// selection alone, which is still read when nothing has been kept yet; and
// that replaced `fleet.lastTerminal`, which named a terminal: the old key is
// read once, mapped by `WorkspaceSelection.mapping`, and removed. A window
// goes back to where it was whatever is waiting on Needs You, as the iPhone
// does (ruling 1); with nowhere kept, it opens on Needs You while anything is
// waiting, rather than on the first terminal wanting attention.

enum SelectionMemory {
    typealias Selection = ContentView.Selection

    /// Where the window was is kept, as a `Destination`'s encoding.
    static let destinationKey = "nav.destination.v1"
    /// Where the last workspace selection was kept by a build before that,
    /// as `encode` writes it: read, never written.
    static let key = "workspace.lastSelection"
    /// The key it replaces: `host/worktree/terminal`.
    static let legacyKey = "fleet.lastTerminal"

    /// `host|workspace|task-or-worktree`, the spec's shape: the third part is
    /// `task:<id>`, `worktree:<id>` or `worktree:<id>:<terminal>`, or empty.
    /// A loose worktree has an empty workspace. Needs You isn't kept: where
    /// the window opens on it is the launch rule's to say.
    static func encode(_ selection: Selection?) -> String? {
        func opened(_ worktree: String, _ terminal: String?) -> String {
            "worktree:\(worktree)" + (terminal.map { ":\($0)" } ?? "")
        }
        switch selection {
        case nil, .needsYou: return nil
        case .workspace(let host, let id, let focus):
            let third: String
            switch focus {
            case nil: third = ""
            case .task(let task): third = "task:\(task)"
            case .worktree(let worktree, let terminal): third = opened(worktree, terminal)
            case .history(let status): third = "history:\(status.rawValue)"
            case .plan(.theme(let theme)): third = "plan:theme:\(theme)"
            case .plan(.lane(let lane)): third = "plan:lane:\(lane)"
            }
            return "\(host)|\(id)|\(third)"
        case .looseWorktree(let host, let worktree, let terminal):
            return "\(host)||\(opened(worktree, terminal))"
        }
    }

    /// `encode`'s string back, or nil for anything else.
    static func decode(_ saved: String) -> Selection? {
        let parts = saved.split(separator: "|", maxSplits: 2, omittingEmptySubsequences: false).map(String.init)
        guard parts.count == 3 else { return nil }
        let (host, workspace, third) = (parts[0], parts[1], parts[2])
        var focus: ContentView.Focus?
        if third.hasPrefix("task:") {
            focus = .task(String(third.dropFirst("task:".count)))
        } else if third.hasPrefix("worktree:") {
            let rest = third.dropFirst("worktree:".count).split(separator: ":", maxSplits: 1).map(String.init)
            guard let id = rest.first, !id.isEmpty else { return nil }
            focus = .worktree(id, terminal: rest.count > 1 ? rest[1] : nil)
        } else if third.hasPrefix("history:") {
            guard let status = TaskStatus(rawValue: String(third.dropFirst("history:".count))) else { return nil }
            focus = .history(status)
        } else if third.hasPrefix("plan:theme:"), third.count > "plan:theme:".count {
            focus = .plan(.theme(String(third.dropFirst("plan:theme:".count))))
        } else if third.hasPrefix("plan:lane:"), third.count > "plan:lane:".count {
            focus = .plan(.lane(String(third.dropFirst("plan:lane:".count))))
        } else if !third.isEmpty {
            return nil
        }
        if workspace.isEmpty {
            guard case .worktree(let id, let terminal)? = focus else { return nil }
            return .looseWorktree(host: host, worktree: id, terminal: terminal)
        }
        return .workspace(host: host, workspace: workspace, focus: focus)
    }

    /// Move `fleet.lastTerminal` to `workspace.lastSelection`, once.
    ///
    /// Waits for the runner the old key names to have been read, since the
    /// mapping needs its fleet: `ready` says whether it has. Then the old key
    /// is removed whatever the mapping found, and a terminal that's gone
    /// leaves nothing behind. A new key already written wins over the old
    /// one. True once there's nothing left to migrate.
    @discardableResult
    static func migrate(_ defaults: UserDefaults, fleet: Fleet, ready: (_ host: String) -> Bool) -> Bool {
        guard let old = defaults.string(forKey: legacyKey) else { return true }
        guard let legacy = LegacySelection(lastTerminal: old) else {
            defaults.removeObject(forKey: legacyKey)
            return true
        }
        guard case .terminal(let host, _, _) = legacy, ready(host) else { return false }
        if defaults.string(forKey: key) == nil,
            let mapped = WorkspaceSelection.mapping(old: legacy, in: fleet),
            let saved = encode(mapped)
        {
            defaults.set(saved, forKey: key)
        }
        defaults.removeObject(forKey: legacyKey)
        return true
    }

    /// Where the window goes back to: what `destination` kept if it reads,
    /// else what `legacy` kept, the selection alone. Nil with neither.
    static func kept(destination: String, legacy: String) -> Destination? {
        if let kept = Destination(encoded: destination) { return kept }
        return decode(legacy).flatMap { MacDestination.destination($0) }
    }

    /// Where a window with nowhere to go back to opens, or nil while that
    /// can't be said yet.
    ///
    /// Needs You when anything is waiting. Otherwise, once every runner
    /// that's answering has said what's waiting (`settled`), the first
    /// workspace, else nothing (`.some(nil)`).
    static func launch(needsYou count: Int, settled: Bool, in fleet: Fleet) -> Selection?? {
        if count > 0 { return .some(.needsYou) }
        guard settled else { return nil }
        return .some(first(in: fleet))
    }

    /// The first workspace the sidebar lists: a repository's Main, on the
    /// first runner with one.
    static func first(in fleet: Fleet) -> Selection? {
        for worktree in fleet.worktrees {
            let host = worktree.host ?? ""
            guard let repository = worktree.repositoryID else { continue }
            if let listed = fleet.runnerWorkspaces[host] {
                if let main = listed.first(where: { $0.repository == repository && $0.isMain })
                    ?? listed.first(where: { $0.repository == repository })
                {
                    return .workspace(host: host, workspace: main.id, focus: nil)
                }
            } else {
                return .workspace(host: host, workspace: repository, focus: nil)
            }
        }
        return nil
    }
}
