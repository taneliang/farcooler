import Foundation

// The phone's Board rows: one per repository that has a board, at the top of
// its runner's section in the shell overview, above that runner's workspaces.
//
// The Mac's sidebar has had this row since card -19 (`BoardRow` in
// `apps/macos/Sources/FarCooler/SidebarViews.swift`), and it draws the same two
// numbers: the tasks waiting on a decision, amber, and a quiet count of the
// tasks an agent is on. The rules for both are AgentKit's already —
// `TaskBoardModel.waitingOnYou`, `tasksWithLiveAgents`, `TaskAgentLink` — and
// what this file adds is which repositories get a row at all, which is a rule
// a view body would otherwise decide and no suite would read.

/// One repository's Board row.
public struct RunnerBoardRow: Equatable, Sendable, Identifiable {
    /// The repository's uuid, which is what `task.list` takes.
    public var repository: String
    /// What to call it: the repository's display name.
    public var name: String
    /// Tasks in Needs Decision. Drawn in amber, and not at all at zero.
    public var decisions: Int
    /// Tasks at least one live agent is on. Drawn quietly, and not at all at
    /// zero — which is also what a runner that cannot say reads as.
    public var agents: Int

    public var id: String { repository }

    public init(repository: String, name: String, decisions: Int, agents: Int) {
        self.repository = repository
        self.name = name
        self.decisions = decisions
        self.agents = agents
    }

    /// What VoiceOver reads after the row's name: the two counts in words,
    /// or nil when neither says anything. The Mac's tooltips, joined.
    public var spoken: String? {
        let parts = [
            TaskBoardModel.decisionsHelp(decisions), TaskBoardModel.agentsHelp(agents),
        ].compactMap { $0 }
        return parts.isEmpty ? nil : parts.joined(separator: ", ")
    }
}

public enum RunnerBoards {
    /// The Board rows for one runner, in the order the runner lists its
    /// repositories.
    ///
    /// - A runner that does not advertise `tasks` has no board, and gets no
    ///   rows. Nor does one nobody has asked yet (`build` nil): a row that
    ///   appeared and then vanished on the first answer would move every card
    ///   under it.
    /// - A repository gets a row only once its board has been read and has
    ///   something on it, unreadable rows included. An empty board is most
    ///   repositories on most runners, and a row for each would push the
    ///   workspaces down for nothing to look at. This is where the phone
    ///   differs from the Mac, whose sidebar row is also the board's only
    ///   way in and so is drawn for an empty one too.
    /// - `agents` is counted only where `TaskAgentLink.speaksOfAgents` says
    ///   the runner can be believed about its panes. Anywhere else it is 0,
    ///   which draws nothing: "can't say", not "none".
    ///
    /// `boards` holds the last good read of each repository's board. It is
    /// kept through a reconnect on purpose, like the fleet: the decisions are
    /// what the runner last said, and a row that blinked out for every
    /// dropped link would be a row nobody could find twice.
    ///
    /// `build` is what THIS link has read, nil from the moment a link comes up
    /// until its `host` answers. `lastKnownBuild` is the last any link read,
    /// kept through a reconnect, and it is what keeps the rows drawn in that
    /// gap: a row that vanished for one round trip after every Wi-Fi blink
    /// would move every card under it up and back down. Agents are counted
    /// only against `build` — the fresh one — so in the gap the rows stay and
    /// say nothing about agents.
    public static func rows<P: TaskBoardPane>(
        repositories: [(id: String, name: String)],
        boards: [String: TaskBoardModel],
        panes: [P],
        build: DaemonBuild?,
        lastKnownBuild: DaemonBuild? = nil,
        connected: Bool
    ) -> [RunnerBoardRow] {
        guard (build ?? lastKnownBuild)?.can("tasks") == true else { return [] }
        let speaks = TaskAgentLink.speaksOfAgents(connected: connected, build: build)
        return repositories.compactMap { repository in
            guard let board = boards[repository.id],
                !board.rows.isEmpty || !board.unreadable.isEmpty
            else { return nil }
            return RunnerBoardRow(
                repository: repository.id, name: repository.name,
                decisions: board.waitingOnYou,
                agents: speaks ? board.tasksWithLiveAgents(in: panes) : 0)
        }
    }
}

extension TaskBoardModel {
    /// The sections a phone's board lists, in `order`: every status with a
    /// task in it, Needs Decision first.
    ///
    /// Only the ones with something in them, which is where a list parts
    /// company with the Mac's columns. Seven fixed columns side by side keep
    /// their places as cards move, and an empty one says "nothing is in
    /// review" at a glance; seven headings down a phone, five of them over
    /// nothing, would put the one card you came for below the fold.
    public var listed: [TaskBoardColumn] {
        columns.filter { !$0.rows.isEmpty }
    }
}
