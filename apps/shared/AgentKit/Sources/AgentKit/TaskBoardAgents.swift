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
    public static let liveStates: Set<String> = ["running", "starting", "unknown"]

    /// Whether `pane` is working the task with id `taskID`.
    ///
    /// Whatever the task's status is. An agent that asked a question moved
    /// its task to `needs_decision` and is still sitting in its pane, which
    /// is the one case where going to it matters most.
    public static func isWorking(_ pane: some TaskBoardPane, on taskID: String) -> Bool {
        guard let id = pane.boardTaskID, !id.isEmpty, id == taskID else { return false }
        return pane.runsAgent && liveStates.contains(pane.boardState)
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
        guard !isChangesPane else { return false }
        let name = preset.split(separator: ":").first.map(String.init) ?? ""
        guard !name.isEmpty else { return false }
        return name != "shell" && !shells.contains(name.lowercased())
    }

    /// The names a plain shell reports itself by.
    static let shells: Set<String> = ["sh", "zsh", "bash", "fish", "dash", "ksh", "-zsh"]

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
        connected && build?.can("terminal_task") == true
    }

    /// What a board says when its Agent button has nowhere to land: the pane
    /// closed after the card was drawn. Said on the board, briefly, instead of
    /// closing it onto nothing. See `ShellFleet.landing`.
    public static let paneHasClosed = "That agent’s pane has closed."

    /// Menu items for several panes on one card, told apart even where their
    /// names are not: titles that collide get the pane's short id after them.
    ///
    /// `titles[i]` and `shorts[i]` are one pane's. The Mac's `BoardPane.titles`
    /// makes the same choice.
    public static func menuTitles(_ titles: [String], shorts: [String]) -> [String] {
        let counts = Dictionary(titles.map { ($0, 1) }, uniquingKeysWith: +)
        return titles.enumerated().map { index, title in
            guard counts[title, default: 0] > 1, shorts.indices.contains(index) else {
                return title
            }
            return "\(title) (\(shorts[index]))"
        }
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
    /// This many panes are working it. Never zero.
    case agents(Int)

    /// The pill's words, or nil for nothing drawn.
    ///
    /// Title case, because it is a control. "Agent" for one rather than "1
    /// Agent": the pill is a button that goes somewhere, and a count on a
    /// button with one destination is arithmetic nobody asked for.
    public var title: String? {
        switch self {
        case .unsaid: return nil
        case .noAgent: return "No Agent"
        case .agents(let n): return n == 1 ? "Agent" : "\(n) Agents"
        }
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

    /// `2 of 5`, or `All 5 met` once they all are.
    ///
    /// Sentence case, because this is a label and not a control — the pill
    /// beside it is the button. Written out rather than formatted, for
    /// `TaskRow.listed`'s reason: the suite's assertions are English. A
    /// single line that holds reads `Met` rather than `All 1 met`, which is a
    /// sentence nobody would write.
    public var sentence: String {
        guard isComplete else { return "\(met) of \(total)" }
        return total == 1 ? "Met" : "All \(total) met"
    }
}

extension TaskRow {
    /// The panes working this task, in the order they were handed in.
    ///
    /// More than one is real: `farcooler task dispatch --again` puts a second
    /// agent on a task, in the same lane or a new one, and the first stays
    /// findable only through its own task id.
    public func livePanes<P: TaskBoardPane>(in panes: [P]) -> [P] {
        panes.filter { TaskAgentLink.isWorking($0, on: id) }
    }

    /// What the card says about its agent.
    ///
    /// `runnerRecordsTasks` is whether the runner advertises `terminal_task`.
    /// Without it no pane carries a task id, so "no agent" would be a claim
    /// this app cannot back: an older runner gets no link and no remark, never
    /// a guess from the task's workspace.
    public func agentPresence(livePanes: Int, runnerRecordsTasks: Bool) -> TaskAgentPresence {
        guard runnerRecordsTasks else { return .unsaid }
        if livePanes > 0 { return .agents(livePanes) }
        return status == .inProgress ? .noAgent : .unsaid
    }

    /// How far along its acceptance this task is, or nil when it has none.
    ///
    /// Nil rather than `0 of 0`, by `waitingSentence`'s rule: a line that is
    /// there on every card whether or not it says anything is a line people
    /// learn to skip.
    public var acceptanceProgress: TaskAcceptanceProgress? {
        guard !acceptance.isEmpty else { return nil }
        return TaskAcceptanceProgress(
            met: acceptance.filter(\.met).count, total: acceptance.count)
    }
}

extension TaskBoardModel {
    /// How many tasks on this board have at least one agent working them.
    ///
    /// Tasks, not panes: the sidebar row answers "how much of this board is
    /// moving", and two agents on one task are still one task moving.
    public func tasksWithLiveAgents<P: TaskBoardPane>(in panes: [P]) -> Int {
        rows.filter { !$0.livePanes(in: panes).isEmpty }.count
    }

    /// The tooltip on the sidebar's amber count, or nil at zero.
    public static func decisionsHelp(_ count: Int) -> String? {
        guard count > 0 else { return nil }
        return count == 1 ? "1 task needs a decision" : "\(count) tasks need a decision"
    }

    /// The tooltip on the sidebar's quiet count, or nil at zero.
    public static func agentsHelp(_ count: Int) -> String? {
        guard count > 0 else { return nil }
        return count == 1 ? "An agent is on 1 task" : "Agents are on \(count) tasks"
    }
}
