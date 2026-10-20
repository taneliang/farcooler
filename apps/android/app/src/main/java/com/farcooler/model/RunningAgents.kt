package com.farcooler.model

// The agents at work, as one list (ov-619, ov-641): Far Cooler's own agents (a lane's pane, a task's pane, an
// orchestrator) and the Claude subagents an agent started, read the same way. The Android twin of AgentKit's
// `RunningAgents.swift`, rule for rule: the Agents card beside Now/Next counts it and the agents view it opens lists it.
//
// Everything here is from the fleet, which every poll and every reconnect replaces whole, so the count is right after a
// reconnect without anything else being asked. A pane says how many subagents it has running
// ([Terminal.subagentsRunning]) and names the ones its current turn started ([Terminal.subagents], each by what it was
// asked, with the call id behind it), and the larger of the two is how many there are: the names go with the turn, while
// agents it started in the background run on. A pane's conversation, when the phone holds it live, adds each named one's
// type, newest call and start, joined on that call id, and fills in the unnamed rest; otherwise the rest is one row
// saying how many more. A row stuck at running never counts an agent that ended, and a conversation that hasn't loaded
// never hides one that runs.

/** A running subagent as its parent's conversation lists it: AgentKit's `AgentTray.Entry`, for the rows that run. */
data class SubagentEntry(
    val id: String,
    val title: String,
    val description: String,
    val running: Boolean = true,
    val startedMs: Long? = null,
    /** The `tool_use` id of the call that started it: what [Terminal.subagentIds] name it by. */
    val toolUseId: String? = null,
    /** What its own conversation is asked for by; null until the runner has named it. */
    val agentId: String? = null,
) {
    companion object {
        /** The subagent rows of a conversation, in order, as entries. */
        fun of(rows: List<AgentRow>): List<SubagentEntry> = rows.mapNotNull { row ->
            val sub = (row.kind as? AgentRow.Kind.OfSubagent)?.subagent ?: return@mapNotNull null
            SubagentEntry(
                id = row.id,
                title = AgentConversation.agentType(sub.agentType),
                description = sub.description,
                running = sub.status == AgentRow.Status.Running,
                startedMs = sub.startedMs,
                toolUseId = sub.toolUseId.ifEmpty { null },
                agentId = sub.agentId,
            )
        }
    }
}

/** One agent at work. */
data class RunningAgent(
    /** Unique across runners: `<runner>/<terminal>`, and a subagent's `<runner>/<terminal>/<its row id or name>`. */
    val id: String,
    val kind: Kind,
    /** The runner's id, as this phone knows it. */
    val runner: String,
    /** The workspace it works for, by id; null for an unclaimed worktree's. */
    val workspace: String?,
    /** "claude", the conversation's own name, or a subagent's type. */
    val name: String,
    /** What it's doing now, in one line; empty when nothing says. */
    val activity: String,
    /** When the current run started, in ms: a pane's turn, a subagent's launch. Null when nothing says. */
    val startedMs: Long?,
    val mark: GlanceMark,
    /** The worktree and pane that open it: its own, or a subagent's parent's. */
    val worktree: String,
    val terminal: String,
    /** A subagent's row id in its parent's conversation; null when only its name is known. */
    val row: String? = null,
    /** How many agents this row stands for: one, or the unnamed rest of a pane's subagents ("3 more agents"). */
    val stands: Int = 1,
    /** Whether its last turn died: its mark is drawn red, at the needs-you weight ([AgentInk.ERROR]). */
    val failed: Boolean = false,
    /** A subagent's id in its parent's conversation, which is what opens its own; null when only its name is known. */
    val agentId: String? = null,
) {
    enum class Kind { ORCHESTRATOR, PANE, SUBAGENT }

    /** Whether choosing it can open the subagent's own conversation rather than its parent's pane. */
    val opensOwnConversation: Boolean get() = kind == Kind.SUBAGENT && row != null && agentId != null
}

/** The agents at work in a scope, in the order they're listed. */
data class RunningAgents(val agents: List<RunningAgent>) {
    /** How many run: every agent listed but the orchestrators, whose conversation is the screen they're counted from. */
    val count: Int get() = agents.filter { it.kind != RunningAgent.Kind.ORCHESTRATOR }.sumOf { it.stands }

    /** The agents the card stacks as dots: the first three it counts, one entry per agent a row stands for. */
    val dotAgents: List<RunningAgent>
        get() = agents.filter { it.kind != RunningAgent.Kind.ORCHESTRATOR }
            .flatMap { agent -> List(agent.stands) { agent } }.take(DOT_LIMIT)

    /** The marks of [dotAgents]. */
    val dots: List<GlanceMark> get() = dotAgents.map { it.mark }

    /** "1 agent", "4 agents": every agent, working, idle or waiting on you, not only the ones running a turn. */
    val summary: String get() = summary(count)

    /** The workspaces [agents] belong to, in order, each with its agents. */
    val byWorkspace: List<Pair<String?, List<RunningAgent>>>
        get() {
            val out = mutableListOf<Pair<String?, MutableList<RunningAgent>>>()
            for (agent in agents) {
                val last = out.lastOrNull()
                if (last != null && last.first == agent.workspace) last.second.add(agent)
                else out.add(agent.workspace to mutableListOf(agent))
            }
            return out
        }

    companion object {
        val EMPTY = RunningAgents(emptyList())
        const val DOT_LIMIT = 3
        const val UNKNOWN_TYPE = "Subagent"
        const val ORCHESTRATOR_NAME = "Orchestrator"

        fun summary(count: Int): String = if (count == 1) "1 agent" else "$count agents"

        /** The unnamed rest of a pane's subagents, in one row. */
        fun more(count: Int): String = if (count == 1) "1 more agent" else "$count more agents"

        /**
         * The agents at work for [workspace] on one runner: its orchestrator's subagents first, then each agent pane, each
         * followed by its own subagents. The orchestrator itself is not listed: this is what it's orchestrating.
         *
         * [rows] is a pane's conversation's running subagents, by terminal, when the phone holds it; null otherwise.
         */
        fun workspace(
            workspace: String,
            runner: String,
            fleet: Fleet,
            rows: (String) -> List<SubagentEntry>? = { null },
        ): RunningAgents {
            val lead = mutableListOf<RunningAgent>()
            val panes = mutableListOf<RunningAgent>()
            for (worktree in fleet.worktrees) {
                for (terminal in worktree.terminals) {
                    if (!isAlive(terminal) || !isAgent(terminal)) continue
                    if (workspaceOf(terminal, worktree) != workspace) continue
                    if (terminal.isOrchestrator) {
                        lead += subagents(terminal, worktree, runner, workspace, rows(terminal.id))
                    } else {
                        panes += pane(terminal, worktree, runner, workspace)
                        panes += subagents(terminal, worktree, runner, workspace, rows(terminal.id))
                    }
                }
            }
            return RunningAgents(lead + panes)
        }

        /** The workspace a pane works for: its own, else its worktree's. */
        fun workspaceOf(terminal: Terminal, worktree: Worktree): String? = terminal.workspace ?: worktree.workspace

        /** An agent's pane rather than a shell's: the runner's role for it, else an agent's activity. */
        fun isAgent(terminal: Terminal): Boolean = terminal.isOrchestrator || terminal.role == "agent" || terminal.agent.isAgent

        /** A pane whose process is still there: not exited, lost or failed. */
        fun isAlive(terminal: Terminal): Boolean = when (StateKind.parse(terminal.state)) {
            StateKind.LOST, StateKind.EXITED, StateKind.ERROR -> false
            else -> true
        }

        private val quietMark = GlanceMark(GlanceMark.Attention.QUIET, GlanceMark.Core.PRODUCING)

        fun pane(terminal: Terminal, worktree: Worktree, runner: String, workspace: String?) = RunningAgent(
            id = "$runner/${terminal.id}", kind = RunningAgent.Kind.PANE, runner = runner, workspace = workspace,
            name = terminal.label, activity = activity(terminal), startedMs = started(terminal),
            // A failed turn wants a person as a blocked one does: the needs-you weight, drawn red (`failed`).
            mark = GlanceMark.of(terminal) ?: GlanceMark(GlanceMark.Attention.NEEDS_YOU, GlanceMark.Core.AT_A_PROMPT),
            failed = agentOutcome(terminal) == AgentOutcome.FAILED,
            worktree = worktree.id, terminal = terminal.id,
        )

        /**
         * [terminal]'s running subagents: as many as the larger of its count and its names. Each is titled by its type
         * ("General purpose", or "Subagent" while nothing says), with what it was asked under it.
         *
         * [rows] is its conversation's running subagents when the phone holds it live. A named agent takes its row by the
         * call id behind its name, or, from a runner that sends no ids, by order; the unnamed rest take the newest rows
         * left over. A row no name or count accounts for is never listed, and the rest no row covers is one row saying how
         * many more.
         */
        fun subagents(
            terminal: Terminal,
            worktree: Worktree,
            runner: String,
            workspace: String?,
            rows: List<SubagentEntry>?,
        ): List<RunningAgent> {
            val names = terminal.subagents.orEmpty()
            val ids = terminal.subagentIds.orEmpty()
            val byId = ids.size == names.size && ids.isNotEmpty()
            val spare = rows.orEmpty().filter { it.running }.toMutableList()

            fun base(id: String, name: String, asked: String) = RunningAgent(
                id = "$runner/${terminal.id}/$id", kind = RunningAgent.Kind.SUBAGENT, runner = runner, workspace = workspace,
                name = name, activity = asked, startedMs = null, mark = quietMark, worktree = worktree.id, terminal = terminal.id,
            )

            fun detailed(agent: RunningAgent, entry: SubagentEntry) = agent.copy(
                id = "$runner/${terminal.id}/${entry.id}",
                name = entry.title,
                activity = entry.description.ifEmpty { agent.activity },
                startedMs = entry.startedMs,
                row = entry.id,
                agentId = entry.agentId,
            )

            val agents = names.mapIndexed { n, name ->
                val agent = base("$name#$n", UNKNOWN_TYPE, name)
                val at = if (byId) spare.indexOfFirst { it.toolUseId == ids[n] } else if (spare.isEmpty()) -1 else 0
                if (at < 0) agent else detailed(agent, spare.removeAt(at))
            }.toMutableList()
            var rest = maxOf(terminal.subagentsRunning ?: 0, names.size) - names.size
            if (rest > 0 && rows != null) {
                val newest = spare.takeLast(rest)
                agents += newest.map { detailed(base(it.id, UNKNOWN_TYPE, ""), it) }
                rest -= newest.size
            }
            if (rest > 0) agents += base("more", more(rest), "").copy(stands = rest)
            return numbered(agents)
        }

        /**
         * Two of one name told apart: "General purpose", "General purpose 2". Not "Subagent": with no type known, what each
         * was asked tells them apart, and "Subagent 4" would only suggest an order nobody gave them.
         */
        fun numbered(agents: List<RunningAgent>): List<RunningAgent> {
            val totals = agents.groupingBy { it.name }.eachCount()
            val seen = HashMap<String, Int>()
            return agents.map { agent ->
                if ((totals[agent.name] ?: 0) <= 1 || agent.name == UNKNOWN_TYPE) return@map agent
                val n = (seen[agent.name] ?: 0) + 1
                seen[agent.name] = n
                if (n > 1) agent.copy(name = "${agent.name} $n") else agent
            }
        }

        /** What a pane is doing: the runner's one line, else its state in a word. */
        fun activity(terminal: Terminal): String = terminal.line?.takeIf { it.isNotEmpty() } ?: terminal.agent.label

        /** When a pane's current run began: its turn, else its state. */
        fun started(terminal: Terminal): Long? {
            terminal.turnStartedAt?.let { return it.toLong() }
            if (terminal.agent != AgentActivity.WORKING) return null
            return terminal.activitySince?.toLong()
        }
    }
}
