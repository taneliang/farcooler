import Foundation

// The agents at work, as one list (ov-619): Far Cooler's own agents (a lane's
// pane, a task's pane, an orchestrator) and the Claude subagents an agent
// started, read the same way. The phone draws it twice from here: the Agents
// card beside Now/Next and the agents view it opens, scoped to a workspace,
// and All Agents on the workspace list, across every runner (ov-617).
//
// Everything here is from the fleet, which every poll and every reconnect
// replaces whole, so the count is right after a reconnect without anything
// else being asked (ov-612's resync covers the rows a pane's conversation
// holds). A pane says how many subagents it has running
// (`Terminal.subagentsRunning`) and names the ones its current turn started
// (`Terminal.subagents`, each by what it was asked, with the call id behind
// it), and the larger of the two is how many there are: the names go with the
// turn, while agents it started in the background run on. A pane's
// conversation, when the phone holds it live, adds each named one's type,
// newest call and start, joined on that call id, and fills in the unnamed
// rest; otherwise the rest is one row saying how many more. A row stuck at
// running never counts an agent that ended, and a conversation that hasn't
// loaded never hides one that runs.

/// One agent at work.
public struct RunningAgent: Identifiable, Equatable, Sendable {
    public enum Kind: Equatable, Sendable {
        /// A workspace's orchestrator.
        case orchestrator
        /// An agent in a pane of its own: a lane's, a task's, or one started by hand.
        case pane
        /// A Claude subagent, running inside `parent`'s pane.
        case subagent
    }

    /// Unique across runners: `<runner>/<terminal>`, and a subagent's
    /// `<runner>/<terminal>/<its row id or name>`.
    public var id: String
    public var kind: Kind
    /// The runner's id, as this phone knows it.
    public var runner: String
    /// The workspace it works for, by id; nil for an unclaimed worktree's.
    public var workspace: String?
    /// "claude", the conversation's own name, or a subagent's type.
    public var name: String
    /// What it's doing now, in one line; empty when nothing says.
    public var activity: String
    /// When the current run started, in ms: a pane's turn, a subagent's
    /// launch. Nil when nothing says.
    public var startedMs: Int64?
    public var mark: GlanceMark
    /// The worktree and pane that open it: its own, or a subagent's parent's.
    public var worktree: String
    public var terminal: String
    /// A subagent's row id and `agentId` in its parent's conversation, which
    /// is what opens its own; nil when only its name is known.
    public var row: String?
    public var agentId: String?
    /// How many agents this row stands for: one, or the unnamed rest of a
    /// pane's subagents when nothing names them ("3 more agents").
    public var stands = 1

    /// Whether choosing it can open the subagent's own conversation rather
    /// than its parent's pane.
    public var opensOwnConversation: Bool { kind == .subagent && row != nil && agentId != nil }
}

/// One agent pane, as a client reads it from its own fleet: what
/// `RunningAgents` needs of it, and nothing the phones' and the Mac's
/// differing `Terminal`s disagree on. The Mac fills one from its own
/// terminal; the phones' is `init(_:in:workspace:)`.
public struct AgentPane: Equatable, Sendable {
    public var terminal: String
    public var worktree: String
    public var label: String
    public var isOrchestrator: Bool
    public var workspace: String?
    /// What it's doing now, in one line; empty when nothing says.
    public var activity: String
    /// When the current turn started, in ms.
    public var startedMs: Int64?
    public var mark: GlanceMark
    /// The subagents its turn named, the call id behind each, and how many
    /// run in all (nil from a runner that doesn't say).
    public var subagents: [String]
    public var subagentIds: [String]
    public var subagentsRunning: Int?

    public init(
        terminal: String, worktree: String, label: String, isOrchestrator: Bool, workspace: String?,
        activity: String, startedMs: Int64?, mark: GlanceMark, subagents: [String], subagentIds: [String],
        subagentsRunning: Int?
    ) {
        self.terminal = terminal
        self.worktree = worktree
        self.label = label
        self.isOrchestrator = isOrchestrator
        self.workspace = workspace
        self.activity = activity
        self.startedMs = startedMs
        self.mark = mark
        self.subagents = subagents
        self.subagentIds = subagentIds
        self.subagentsRunning = subagentsRunning
    }
}

extension AgentPane {
    init(_ terminal: Terminal, in worktree: Worktree, workspace: String?) {
        self.init(
            terminal: terminal.id, worktree: worktree.id, label: terminal.label, isOrchestrator: terminal.isOrchestrator,
            workspace: workspace, activity: RunningAgents.activity(terminal), startedMs: RunningAgents.started(terminal),
            mark: GlanceMark(terminal: terminal), subagents: terminal.subagents ?? [],
            subagentIds: terminal.subagentIds ?? [], subagentsRunning: terminal.subagentsRunning)
    }
}

/// The agents at work in a scope, in the order they're listed.
public struct RunningAgents: Equatable, Sendable {
    public var agents: [RunningAgent]

    public init(agents: [RunningAgent]) { self.agents = agents }

    public static let empty = RunningAgents(agents: [])

    /// How many run: every agent listed but the orchestrators, whose
    /// conversation is the screen they're counted from.
    public var count: Int { agents.filter { $0.kind != .orchestrator }.map(\.stands).reduce(0, +) }

    /// The marks the card stacks: the first three it counts.
    public var dots: [GlanceMark] {
        let counted = agents.filter { $0.kind != .orchestrator }
        return Array(counted.flatMap { Array(repeating: $0.mark, count: $0.stands) }.prefix(Self.dotLimit))
    }

    public static let dotLimit = 3

    /// "1 agent", "4 agents": every agent, working, idle or waiting on
    /// you, not only the ones running a turn.
    public var summary: String { Self.summary(count) }

    public static func summary(_ count: Int) -> String {
        count == 1 ? "1 agent" : "\(count) agents"
    }

    /// The agents at work for `workspace` on one runner: its orchestrator's
    /// subagents first, then each agent pane, each followed by its own
    /// subagents. The orchestrator itself is not listed: this is what it's
    /// orchestrating.
    ///
    /// `rows` is a pane's conversation's tray (`AgentTray.entries`), by
    /// terminal, when the phone holds it; nil otherwise.
    static func workspace(
        _ workspace: String, runner: String, fleet: Fleet, rows: (String) -> [AgentTray.Entry]? = { _ in nil }
    ) -> RunningAgents {
        var panes: [AgentPane] = []
        for worktree in fleet.worktrees {
            for terminal in worktree.terminals where isAlive(terminal) && isAgent(terminal) {
                guard fleet.workspace(of: terminal, in: worktree) == workspace else { continue }
                panes.append(AgentPane(terminal, in: worktree, workspace: workspace))
            }
        }
        return self.workspace(workspace, runner: runner, panes: panes, rows: rows)
    }

    /// The same, from panes a client has already read out of its own fleet:
    /// the agent panes of `workspace` that are alive, in the fleet's order.
    /// What the Mac calls, whose fleet is not the phones'.
    public static func workspace(
        _ workspace: String, runner: String, panes: [AgentPane], rows: (String) -> [AgentTray.Entry]? = { _ in nil }
    ) -> RunningAgents {
        var out: [RunningAgent] = []
        var listed: [RunningAgent] = []
        for pane in panes {
            if pane.isOrchestrator {
                out += subagents(of: pane, runner: runner, rows: rows(pane.terminal))
            } else {
                listed.append(Self.pane(pane, runner: runner))
                listed += subagents(of: pane, runner: runner, rows: rows(pane.terminal))
            }
        }
        return RunningAgents(agents: out + listed)
    }

    /// One runner's part of All Agents: each workspace's orchestrator with
    /// its subagents, then its agent panes with theirs, workspaces in the
    /// fleet's order; agents in no workspace last.
    static func runner(_ runner: String, fleet: Fleet, rows: (String) -> [AgentTray.Entry]? = { _ in nil }) -> RunningAgents {
        var groups: [(workspace: String?, lead: [RunningAgent], rest: [RunningAgent])] = []
        func group(_ workspace: String?) -> Int {
            if let found = groups.firstIndex(where: { $0.workspace == workspace }) { return found }
            groups.append((workspace, [], []))
            return groups.count - 1
        }
        for worktree in fleet.worktrees {
            for terminal in worktree.terminals where isAlive(terminal) && isAgent(terminal) {
                let workspace = fleet.workspace(of: terminal, in: worktree)
                let at = group(workspace)
                let source = AgentPane(terminal, in: worktree, workspace: workspace)
                let subs = subagents(of: source, runner: runner, rows: rows(terminal.id))
                if terminal.isOrchestrator {
                    // The orchestrator leads its workspace's list.
                    groups[at].lead += [orchestrator(source, runner: runner)] + subs
                } else {
                    groups[at].rest += [pane(source, runner: runner)] + subs
                }
            }
        }
        let ordered = groups.filter { $0.workspace != nil } + groups.filter { $0.workspace == nil }
        return RunningAgents(agents: ordered.flatMap { $0.lead + $0.rest })
    }

    /// The workspaces `agents` belong to, in order, each with its agents.
    public var byWorkspace: [(workspace: String?, agents: [RunningAgent])] {
        var out: [(workspace: String?, agents: [RunningAgent])] = []
        for agent in agents {
            if let last = out.indices.last, out[last].workspace == agent.workspace {
                out[last].agents.append(agent)
            } else {
                out.append((agent.workspace, [agent]))
            }
        }
        return out
    }

    // MARK: - Pieces

    /// An agent's pane rather than a shell's: the runner's role for it, else
    /// an agent's activity.
    static func isAgent(_ terminal: Terminal) -> Bool {
        terminal.isOrchestrator || terminal.role == "agent" || terminal.agent.isAgent
    }

    /// A pane whose process is still there: not exited, lost or failed.
    static func isAlive(_ terminal: Terminal) -> Bool {
        switch StateKind.parse(terminal.state) {
        case .lost, .exited, .error: false
        default: true
        }
    }

    static func pane(_ source: AgentPane, runner: String) -> RunningAgent {
        RunningAgent(
            id: "\(runner)/\(source.terminal)", kind: .pane, runner: runner, workspace: source.workspace,
            name: source.label, activity: source.activity, startedMs: source.startedMs,
            mark: source.mark, worktree: source.worktree, terminal: source.terminal)
    }

    static func orchestrator(_ source: AgentPane, runner: String) -> RunningAgent {
        var agent = pane(source, runner: runner)
        agent.kind = .orchestrator
        agent.name = orchestratorName
        return agent
    }

    /// `pane`'s running subagents: as many as the larger of its count and
    /// its names. Each is titled by its type ("General purpose", or
    /// "Subagent" while nothing says), with what it was asked under it.
    ///
    /// `rows` is its conversation's tray when the phone holds it live. A named
    /// agent takes its row by the call id behind its name, or, from a runner
    /// that sends no ids, by order; the unnamed rest take the newest rows left
    /// over. A row no name or count accounts for is never listed, and the
    /// rest no row covers is one row saying how many more.
    static func subagents(of pane: AgentPane, runner: String, rows: [AgentTray.Entry]?) -> [RunningAgent] {
        let names = pane.subagents
        let ids = pane.subagentIds
        let byId = ids.count == names.count && !ids.isEmpty
        var spare = (rows ?? []).filter { !$0.isMain && $0.running }
        func base(_ id: String, name: String, asked: String) -> RunningAgent {
            RunningAgent(
                id: "\(runner)/\(pane.terminal)/\(id)", kind: .subagent, runner: runner, workspace: pane.workspace,
                name: name, activity: asked, startedMs: nil, mark: GlanceMark(attention: .quiet, core: .producing),
                worktree: pane.worktree, terminal: pane.terminal)
        }
        func detailed(_ agent: RunningAgent, _ entry: AgentTray.Entry) -> RunningAgent {
            var out = agent
            out.id = "\(runner)/\(pane.terminal)/\(entry.id)"
            out.name = entry.title
            if !entry.description.isEmpty { out.activity = entry.description }
            out.startedMs = entry.startedMs
            out.row = entry.id
            out.agentId = entry.agentId
            return out
        }
        var agents: [RunningAgent] = names.enumerated().map { n, name in
            let agent = base("\(name)#\(n)", name: unknownType, asked: name)
            let at = byId ? spare.firstIndex { $0.toolUseId == ids[n] } : spare.indices.first
            guard let at else { return agent }
            return detailed(agent, spare.remove(at: at))
        }
        var rest = max(pane.subagentsRunning ?? 0, names.count) - names.count
        if rest > 0, rows != nil {
            let newest = spare.suffix(rest)
            agents += newest.map { detailed(base($0.id, name: unknownType, asked: ""), $0) }
            rest -= newest.count
        }
        if rest > 0 {
            var more = base("more", name: Self.more(rest), asked: "")
            more.stands = rest
            agents.append(more)
        }
        return numbered(agents)
    }

    /// A subagent's title while nothing says its type.
    public static let unknownType = "Subagent"

    /// The unnamed rest of a pane's subagents, in one row.
    public static func more(_ count: Int) -> String {
        count == 1 ? "1 more agent" : "\(count) more agents"
    }

    /// Two of one name told apart: "General purpose", "General purpose 2".
    /// Not "Subagent": with no type known, what each was asked tells them
    /// apart, and "Subagent 4" would only suggest an order nobody gave them.
    static func numbered(_ agents: [RunningAgent]) -> [RunningAgent] {
        let totals = Dictionary(agents.map { ($0.name, 1) }, uniquingKeysWith: +)
        var seen: [String: Int] = [:]
        return agents.map { agent in
            guard totals[agent.name, default: 0] > 1, agent.name != unknownType else { return agent }
            let n = seen[agent.name, default: 0] + 1
            seen[agent.name] = n
            var out = agent
            if n > 1 { out.name = "\(agent.name) \(n)" }
            return out
        }
    }

    /// What a pane is doing: the runner's one line, else its state in a word.
    static func activity(_ terminal: Terminal) -> String {
        if let line = terminal.line, !line.isEmpty { return line }
        return terminal.agent.label
    }

    /// When a pane's current run began: its turn, else its state.
    static func started(_ terminal: Terminal) -> Int64? {
        if let turn = terminal.turnStartedAt { return Int64(turn) }
        guard terminal.agent == .working, let since = terminal.activitySince else { return nil }
        return Int64(since)
    }

    static let orchestratorName = "Orchestrator"
}
