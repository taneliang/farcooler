import Foundation

// Which agent is working which card, and how far along a card is.
//
// Beside `TaskBoardModel` and for its reason: every rule here decides a
// sentence or a link somebody acts on, and a rule that lives in a view body is
// one no suite calls. The Mac and the iPhone both draw these, which is the
// other reason they are here: one rule, so the two agree about the same card.
// `swift test --package-path apps/shared/AgentKit` is what checks them.
//
// ## Where "working" comes from
//
// The runner records which task a terminal was opened for (`terminals.task_id`,
// set by `farcooler task dispatch` and `terminal new --task`). Both of the
// CLI's terminal projections carry it as `taskId` for the Mac, and so does the
// client core's fleet (`Session::fleet`) for the phone. The rule for whether such a
// pane is still ON the task is the CLI's own `working_on`
// (crates/cli/src/tasks.rs): the task id matches and the pane is running,
// starting, or unknown. This adds one clause the CLI does not need: the pane
// has to be running an agent. A dispatched pane whose agent exited drops back
// to its shell and stays `running`, and a card offering to take you to a bare
// prompt would be a link to the place the work used to be.

/// A terminal, as much of it as the board needs to decide whether it is
/// working a task.
///
/// A protocol rather than a type, because the Mac's `Terminal` is not
/// AgentKit's: the Mac decodes the CLI's JSON into its own model, and the rule
/// has to be asked of that model and not of a copy made for the purpose.
public protocol TaskBoardPane {
    /// The board task this pane was opened for, as the uuid a `TaskRow`
    /// carries, or nil for a pane nobody dispatched.
    var boardTaskID: String? { get }
    /// The runner's own word for the process: `running`, `starting`,
    /// `exited`, `error`, `LOST`, `unknown`.
    var boardState: String { get }
    /// Whether an agent is what is running in it: not a shell, and not a
    /// changes pane.
    var runsAgent: Bool { get }
}

public enum TaskAgentLink {
    /// The states in which a pane still counts as on its task.
    ///
    /// The CLI's `working_on` list, in the words `terminal_label` prints them
    /// in. `unknown` is in it on purpose: it is the runner being unable to
    /// read the pane for a moment, and that usually resolves with the agent
    /// still there — dropping the link for it would make the pill flicker.
    public static var liveStates: Set<String> { TaskBoardPresenter.settings?.liveStates ?? [] }

    /// Whether `pane` is working the task with id `taskID`.
    ///
    /// Whatever the task's status is. An agent that asked a question moved
    /// its task to `needs_decision` and is still sitting in its pane, which
    /// is the one case where going to it matters most.
    public static func isWorking(_ pane: some TaskBoardPane, on taskID: String) -> Bool {
        !livePanes([pane], on: taskID).isEmpty
    }

    fileprivate static func livePanes<P: TaskBoardPane>(_ panes: [P], on task: String) -> [P] {
        let indices: [Int] = TaskBoardPresenter.ask("live_panes", [
            "task": task, "panes": panes.map { paneInput($0) },
        ]) ?? []
        return indices.map { panes[$0] }
    }

    static func paneInput(_ pane: any TaskBoardPane) -> [String: Any] {
        let task = pane.boardTaskID ?? ""
        return ["task": task, "state": pane.boardState, "agent": !task.isEmpty && pane.runsAgent]
    }
}

extension TaskAgentLink {
    /// Whether a pane running `preset` is running an agent: not a shell, and
    /// not a changes pane.
    ///
    /// The Mac's `Terminal.runsAgent` and the phone's both ask this, so the two
    /// agree about the same pane. `preset` is what the runner reports is
    /// running — `claude`, `codex:gpt-5`, `zsh` — and a live plain shell
    /// reports its real process name rather than the word `shell`, so every
    /// shell's name is refused here, not only that one. A changes pane runs
    /// `farcooler`, which is not a shell and is not an agent either: the
    /// daemon never dispatches one for a task, and a pill that could lead to a
    /// diff would be a rule that works by luck.
    public static func runsAgent(preset: String, isChangesPane: Bool) -> Bool {
        TaskBoardPresenter.ask("runs_agent", ["preset": preset, "changes": isChangesPane]) ?? false
    }

    /// Whether a board may say anything about agents on one runner.
    ///
    /// Two gates, the Mac's `BoardAgents.on` rule. The runner has to record
    /// which pane works which task (`terminal_task`), and it has to be
    /// connected right now: connecting, reconnecting and unreachable all mean
    /// the panes are the last ones read before the link went, kept so the
    /// screen stays put, and the agents in them may have exited since. So a
    /// runner that is not connected gets no pills, no "No Agent", and no count
    /// — "can't say", which is different from "none".
    public static func speaksOfAgents(connected: Bool, build: DaemonBuild?) -> Bool {
        TaskBoardPresenter.ask("show_agents", ["connected": connected, "recorded": build?.can(.terminalTask) == true]) ?? false
    }

    /// What a board says when its Agent button has nowhere to land: the pane
    /// closed after the card was drawn. Said on the board, briefly, instead of
    /// closing it onto nothing. See `ShellFleet.landing`.
    public static let paneHasClosed: String = TaskBoardPresenter.ask("closed_pane") ?? ""

    /// Menu items for several panes on one card, told apart even where their
    /// names are not: titles that collide get the pane's short id after them.
    ///
    /// `titles[i]` and `shorts[i]` are one pane's. The Mac's `BoardPane.titles`
    /// makes the same choice.
    public static func menuTitles(_ titles: [String], shorts: [String]) -> [String] {
        TaskBoardPresenter.ask("menu_titles", ["titles": titles, "shorts": shorts]) ?? titles
    }
}

/// What a card says about the agent on it.
public enum TaskAgentPresence: Equatable, Sendable {
    /// Say nothing: no agent on it, and no reason to remark on that.
    case unsaid
    /// In progress, on a runner that records which pane works which task,
    /// and no pane is working it. Said quietly — it is the "assumed in
    /// flight" state the board exists to catch, but it is not a question for
    /// the person reading the board.
    case noAgent
    /// This many panes are working it. Never zero. With subagents as well,
    /// the count is both, and the control still goes to the panes.
    case agents(Int)
    /// This many subagents are working it, from inside another agent's
    /// session (ov-213), and no pane is. Never zero. The control goes to the
    /// orchestrator's pane, where they live.
    case subagents(Int, SubagentState)

    /// The pill's words, or nil for nothing drawn.
    ///
    /// Title case, because it is a control. "Agent" for one rather than "1
    /// Agent": the pill is a button that goes somewhere, and a count on a
    /// button with one destination is arithmetic nobody asked for.
    public var title: String? {
        let kind: String
        let count: Int
        switch self {
        case .unsaid: return nil
        case .noAgent: (kind, count) = ("no_agent", 0)
        case .agents(let n): (kind, count) = ("agents", n)
        case .subagents(let n, _): (kind, count) = ("subagents", n)
        }
        return TaskBoardPresenter.ask("agent_title", ["kind": kind, "count": count])
    }
}

/// How much of a card's acceptance holds.
public struct TaskAcceptanceProgress: Equatable, Sendable {
    public var met: Int
    public var total: Int

    public init(met: Int, total: Int) {
        self.met = met
        self.total = total
    }

    /// Whether every line is met. Drawn in the accent color, because it is
    /// the one state of this line that says "ready to look at".
    public var isComplete: Bool { total > 0 && met == total }

    public var sentence: String {
        TaskBoardPresenter.ask("acceptance_words", ["met": met, "total": total]) ?? ""
    }
}

extension TaskRow {
    /// The panes working this task, in the order they were handed in.
    ///
    /// More than one is real: `farcooler task dispatch --again` puts a second
    /// agent on a task, in the same lane or a new one, and the first stays
    /// findable only through its own task id.
    public func livePanes<P: TaskBoardPane>(in panes: [P]) -> [P] {
        TaskAgentLink.livePanes(panes, on: id)
    }

    /// What the card says about its agent.
    ///
    /// A pane or a subagent is an agent. "No Agent" is only for a started
    /// card with neither and nothing on the board explaining it: every lane
    /// in a project run by an orchestrator is a subagent with no pane, and an
    /// alarm that's always on tells you nothing (ov-213).
    ///
    /// `runnerRecordsTasks` is whether the runner advertises `terminal_task`.
    /// Without it no pane carries a task id, so "no agent" would be a claim
    /// this app cannot back: an older runner gets no link and no remark, never
    /// a guess from the task's worktree.
    public func agentPresence(livePanes: Int, runnerRecordsTasks: Bool) -> TaskAgentPresence {
        struct Presence: Decodable {
            let kind: String
            let count: Int?
            let state: SubagentState?
        }
        var fields = workerFields
        fields["live"] = livePanes
        fields["recorded"] = runnerRecordsTasks
        fields["status"] = status.rawValue
        if case .inLine(let line, let position)? = wait {
            fields["line"] = line.rawValue
            fields["position"] = position
        }
        let view: Presence? = TaskBoardPresenter.ask("agent_presence", fields)
        switch view?.kind {
        case "agents": return .agents(view?.count ?? livePanes)
        case "subagents":
            guard let count = view?.count, let state = view?.state else { return .unsaid }
            return .subagents(count, state)
        case "no_agent": return .noAgent
        default: return .unsaid
        }
    }

    /// How far along its acceptance this task is, or nil when it has none.
    ///
    /// Nil rather than `0 of 0`, by `waitingSentence`'s rule: a line that is
    /// there on every card whether or not it says anything is a line people
    /// learn to skip.
    public var acceptanceProgress: TaskAcceptanceProgress? {
        struct Progress: Decodable {
            let met: Int
            let total: Int
        }
        let progress: Progress? = TaskBoardPresenter.ask("acceptance", ["lines": acceptance.map(\.met)])
        return progress.map { TaskAcceptanceProgress(met: $0.met, total: $0.total) }
    }
}

extension TaskBoardModel {
    /// How many tasks on this board have at least one agent working them.
    ///
    /// Tasks, not panes: the sidebar row answers "how much of this board is
    /// moving", and two agents on one task are still one task moving.
    public func tasksWithLiveAgents<P: TaskBoardPane>(in panes: [P]) -> Int {
        TaskBoardPresenter.ask("live_task_count", [
            "tasks": rows.map(\.id), "panes": panes.map { TaskAgentLink.paneInput($0) },
        ]) ?? 0
    }

    /// The tooltip on the sidebar's amber count, or nil at zero.
    public static func decisionsHelp(_ count: Int) -> String? {
        TaskBoardPresenter.ask("decisions", ["count": count])
    }

    /// The tooltip on the sidebar's quiet count, or nil at zero.
    public static func agentsHelp(_ count: Int) -> String? {
        TaskBoardPresenter.ask("agents", ["count": count])
    }
}

/// How an agent's harness is spelled where a person reads it.
///
/// The runner names a pane for the command in it, so a Claude pane is "claude".
/// That is the right key and the wrong spelling for a title: the composer asks
/// "Message Claude" under a header reading "claude". Only the harnesses this
/// app launches are mapped, and only the first word, so a pane someone named
/// "claude in fix-reconnect" or "claude 2" keeps its own words and "tmux" is not
/// dressed up as a product.
public enum HarnessName {
    private static let known: [String: String] = [
        "claude": "Claude", "codex": "Codex", "cursor": "Cursor",
    ]

    public static func display(_ name: String) -> String {
        let first = name.prefix { $0 != " " }
        guard let proper = known[String(first)] else { return name }
        return proper + name.dropFirst(first.count)
    }
}
