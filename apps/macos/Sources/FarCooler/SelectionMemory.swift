import AgentKit
import Foundation

// Where the window reopens (spec §4.6 and §9).
//
// The last workspace selection is kept in `workspace.lastSelection`. It
// replaces `fleet.lastTerminal`, which named a terminal: the old key is read
// once, mapped by `WorkspaceSelection.mapping`, and removed. And the window
// opens on Needs You while anything is waiting, as the iPhone does (ruling
// 4), rather than on the first terminal wanting attention.

enum SelectionMemory {
    typealias Selection = ContentView.Selection

    /// Where the last workspace selection is kept, as `encode` writes it.
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

    /// Where the window opens, or nil while that can't be said yet.
    ///
    /// Needs You when anything is waiting. Otherwise, once every runner
    /// that's answering has said what's waiting (`settled`), the last
    /// workspace selection if it's still there (`restored`), else the first
    /// workspace the sidebar lists, else nothing (`.some(nil)`).
    static func launch(needsYou count: Int, settled: Bool, last: Selection?, in fleet: Fleet) -> Selection?? {
        if count > 0 { return .some(.needsYou) }
        guard settled else { return nil }
        if let last, let restored = restored(last, in: fleet) { return .some(restored) }
        return .some(first(in: fleet))
    }

    /// `last` as the fleet has it now: its workspace still there, with a
    /// worktree it opened closed again when that's gone. A task is kept: the
    /// task column says so when it's gone. Nil when the workspace or the
    /// loose worktree is gone.
    static func restored(_ last: Selection, in fleet: Fleet) -> Selection? {
        switch last {
        case .needsYou:
            return nil
        case .looseWorktree(let host, let id, let terminal):
            guard let worktree = WorkspaceSelection.worktree(host: host, id: id, in: fleet) else { return nil }
            let live = terminal.flatMap { t in worktree.terminals.contains { $0.id == t } ? t : nil }
            return .looseWorktree(host: host, worktree: id, terminal: live)
        case .workspace(let host, let id, let focus):
            let exists: Bool = {
                if let listed = fleet.runnerWorkspaces[host] { return listed.contains { $0.id == id } }
                return fleet.worktrees.contains { ($0.host ?? "") == host && $0.repositoryID == id }
            }()
            guard exists else { return nil }
            if case .worktree(let worktree, _)? = focus,
                WorkspaceSelection.worktree(host: host, id: worktree, in: fleet) == nil
            {
                return .workspace(host: host, workspace: id, focus: nil)
            }
            return last
        }
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
