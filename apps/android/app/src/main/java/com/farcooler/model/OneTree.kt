package com.farcooler.model

/**
 * The One tree on an Android phone (ov-300; ov-321's Concept 1): the
 * workspace's work as one outline that follows containment,
 *
 *     Theme › Task › Lane › Terminals
 *
 * where a lane is its worktree. A port of AgentKit's `OneTree` and
 * `PhoneTree` (apps/shared/AgentKit/Sources/AgentKit/OneTree.swift,
 * PhoneTree.swift), the rules the Mac's sidebar and the iPhone's push
 * navigation read, kept to what a phone draws: the themes in plan order with
 * their open cards and a fold of done ones, No Theme, then Main checkout and
 * Loose worktrees. Each card's lanes, each lane's terminals and its
 * subagents, a lane under two cards marked "also", and an ask's dot that rolls
 * up to whatever closes over it. The pinned places are the phone's already:
 * Needs you is the app's front door and the strip's count, and the plan is
 * the sheet.
 *
 * Copy is sentence case, as Android's is.
 */
object OneTree {
    /** Which cards the tree shows. */
    enum class Filter(val title: String) {
        OPEN("Not done"),
        IN_REVIEW("In review"),
        ALL("All");

        fun shows(status: TaskStatus): Boolean = when (this) {
            OPEN -> !status.isFinished
            IN_REVIEW -> status == TaskStatus.IN_REVIEW
            ALL -> true
        }

        companion object {
            fun parse(name: String?): Filter = entries.firstOrNull { it.name == name } ?: OPEN
        }
    }

    enum class Kind { THEME, TASK, DONE_FOLD, LANE, TERMINAL, SUBAGENT, PAGE, GROUP, WORKTREE }

    /** Where a node goes when it's chosen. */
    sealed interface Target {
        data class Theme(val id: String) : Target
        data class Task(val id: String) : Target
        data class Lane(val id: String) : Target
        data class Worktree(val id: String) : Target
        data class Terminal(val worktree: String, val terminal: String) : Target
        /** An orchestrator's page, by slot. */
        data class Page(val slot: String) : Target
        /** A subagent's: it has no pane, and runs inside the orchestrator. */
        data object Orchestrator : Target
    }

    data class Node(
        /** Unique in the tree: its path from the root. */
        val id: String,
        val kind: Kind,
        val title: String,
        val key: String = "",
        val detail: String = "",
        val caption: String = "",
        val also: String = "",
        val target: Target? = null,
        val children: List<Node> = emptyList(),
        val asks: Boolean = false,
        val quiet: Boolean = false,
        val worktreeId: String? = null,
    ) {
        /** It or anything under it asks: what a closed row's dot rolls up. */
        val holdsAsk: Boolean get() = asks || children.any { it.holdsAsk }

        /** On a phone every row with children is closed behind its push. */
        val showsDot: Boolean get() = holdsAsk

        val hasChildren: Boolean get() = children.isNotEmpty()
    }

    data class Tree(val work: List<Node>, val below: List<Node>) {
        val all: List<Node> get() = buildList { fun walk(n: List<Node>) { n.forEach { add(it); walk(it.children) } }; walk(work + below) }

        fun node(id: String): Node? = all.firstOrNull { it.id == id }
    }

    /** What a tapped row does. */
    sealed interface Tap {
        data class Push(val node: String) : Tap
        data class Open(val target: Target) : Tap
        data object None : Tap
    }

    /**
     * The loose worktrees drawn either side of [nodeId]'s row, by worktree id:
     * what Move up and Move down pass. Only a loose worktree moves; a lane is
     * where its card puts it.
     */
    fun neighbors(tree: Tree, nodeId: String): Pair<String?, String?> {
        val siblings = tree.all.firstOrNull { p -> p.children.any { it.id == nodeId } }?.children ?: return null to null
        val row = siblings.firstOrNull { it.id == nodeId } ?: return null to null
        val loose = siblings.filter { it.kind == Kind.WORKTREE && it.worktreeId != null }
        val at = loose.indexOf(row)
        // A lane, a card or a terminal isn't among them, and doesn't move.
        if (at < 0) return null to null
        return loose.getOrNull(at - 1)?.worktreeId to loose.getOrNull(at + 1)?.worktreeId
    }

    /** Where one read the tree is built from stands. */
    enum class Read { PENDING, READ, FAILED, NOT_KEPT }

    /** What a pushed level draws. */
    enum class LevelState { NODE, LOADING, FAILED, GONE }

    /**
     * A level found draws; one not found waits for the board and the plan,
     * and is gone only once both have come back without it (ov-300 review 1).
     * AgentKit's `PhoneTree.level`.
     */
    fun level(found: Boolean, board: Read, plan: Read): LevelState = when {
        found -> LevelState.NODE
        board == Read.FAILED || plan == Read.FAILED -> LevelState.FAILED
        board == Read.PENDING || plan == Read.PENDING -> LevelState.LOADING
        else -> LevelState.GONE
    }

    /** A row with children pushes its level; a leaf opens its target; a subagent opens nothing. */
    fun tap(node: Node): Tap = when {
        node.hasChildren -> Tap.Push(node.id)
        node.target == null || node.target == Target.Orchestrator -> Tap.None
        else -> Tap.Open(node.target)
    }

    /** The first row of a level: the node's own page. Null for a group with none. */
    fun ownRow(node: Node): String? = when (node.target) {
        is Target.Theme -> "Theme page"
        is Target.Task -> "Task details"
        is Target.Lane -> "Lane page"
        is Target.Worktree -> "Open worktree"
        else -> null
    }

    object Words {
        const val NO_THEME = "No theme"
        const val MAIN_CHECKOUT = "Main checkout"
        const val LOOSE_WORKTREES = "Loose worktrees"
        const val SUBAGENT_CAPTION = "No terminal; runs inside the orchestrator"
        const val HIDDEN = "Hidden"
        const val NOT_ON_THIS_VERSION = "Not on this version"
        const val NOT_ON_THIS_VERSION_CAPTION = "This runner uses states this Far Cooler doesn’t have yet."
        fun done(n: Int) = "$n done"
        fun also(keys: List<String>): String = when (keys.size) {
            0 -> ""
            1 -> "also ${keys[0]}"
            else -> "also ${keys[0]} +${keys.size - 1}"
        }
        fun shells(n: Int) = when (n) { 0 -> "No shells"; 1 -> "1 shell"; else -> "$n shells" }
        fun role(raw: String) = when (raw) { "review" -> "Reviewer"; "fix" -> "Fixer"; else -> "Builder" }
        fun subagent(role: String, model: String): String = PlanWords.model(model).let { if (it.isEmpty()) role else "$role · $it" }
        fun progress(c: PlanCounts) = "${c.done}/${PlanWords.total(c)}"
    }

    /** A card, as the tree needs it. */
    data class Card(val id: String, val key: String, val title: String, val status: TaskStatus, val activityMs: Long = 0, val worktreeId: String? = null, val workers: List<TaskWorker> = emptyList())

    /**
     * [workspace]'s tree: its [board]'s cards, its [plan], the runner's
     * [worktrees], and its own Needs You [items] for the dots.
     */
    fun build(
        workspace: WorkspaceSummary,
        board: TaskBoard?,
        plan: Plan,
        worktrees: List<Worktree>,
        items: List<NeedsYouItem>,
        filter: Filter,
        /** The board's orchestrator pages: each anchored one hangs under its theme. */
        pages: List<BoardPage> = emptyList(),
    ): Tree = Builder(workspace, board, plan, worktrees, items, filter, pages).build()

    /** Card statuses as the plan's CLI words them: "In Review", "in_review". */
    internal fun status(word: String): TaskStatus? {
        TaskStatus.parse(word)?.let { return it }
        val folded = word.lowercase().replace(" ", "").replace("_", "")
        if (folded == "cancelled" || folded == "canceled") return TaskStatus.CANCELLED
        return TaskStatus.entries.firstOrNull { it.wire.replace("_", "") == folded }
    }

    private class Builder(
        val workspace: WorkspaceSummary,
        board: TaskBoard?,
        val plan: Plan,
        allWorktrees: List<Worktree>,
        items: List<NeedsYouItem>,
        val filter: Filter,
        val pages: List<BoardPage>,
    ) {
        val rows = board?.rows.orEmpty()
        val unreadable = board?.unreadable.orEmpty()
        val cards: Map<String, Card>
        val themes = plan.shownThemes
        val lanes = plan.lanes.filter { it.state != LaneState.DROPPED }
        val lanesByTask: Map<String, List<PlanLane>>
        val worktrees: List<Worktree>
        val checkout: Worktree?
        val laneWorktrees: Map<String, Worktree>
        val askTasks: Set<String>
        val askTerminals: Set<String>
        val askThemes: Set<String> = themes.filter { it.ownerAsk.isNotEmpty() }.map { it.id }.toSet()

        init {
            val byId = linkedMapOf<String, Card>()
            rows.forEach { byId.putIfAbsent(it.id, Card(it.id, it.key, it.title, it.status, it.lastMovedMs, it.worktreeId, it.workers)) }
            plan.cards.forEach { c -> status(c.status)?.let { byId.putIfAbsent(c.task, Card(c.task, c.key, c.title, it)) } }
            cards = byId
            val byTask = mutableMapOf<String, MutableList<PlanLane>>()
            lanes.forEach { lane -> lane.cards.map { it.task }.distinct().forEach { byTask.getOrPut(it) { mutableListOf() }.add(lane) } }
            lanesByTask = byTask
            val repository = workspace.repository ?: workspace.id
            checkout = allWorktrees.firstOrNull { it.isMainCheckout && it.repository == repository }
            worktrees = allWorktrees.filter { wt ->
                if (wt.isMainCheckout) return@filter false
                val owned = if (workspace.isImplicit) wt.repository == workspace.id else wt.workspace == workspace.id
                owned || taskIds(wt).any { it in cards }
            }
            laneWorktrees = lanes.mapNotNull { lane -> join(lane)?.let { lane.id to it } }.toMap()
            val mine = items.filter {
                if (workspace.isImplicit) it.workspaceId == null && it.repositoryId == workspace.id else it.workspaceId == workspace.id
            }
            askTasks = mine.mapNotNull { it.task?.id }.toSet()
            askTerminals = mine.filter { it.task == null }.mapNotNull { it.terminal?.id }.toSet()
        }

        fun taskIds(wt: Worktree) = wt.openTasks.map { it.id } + wt.terminals.mapNotNull { it.taskId }

        /** A lane's worktree: at its path, by a path ending with it, else on its branch. */
        fun join(lane: PlanLane): Worktree? {
            val path = lane.worktreePath.trim('/')
            if (path.isNotEmpty()) {
                worktrees.firstOrNull { it.worktree == lane.worktreePath }?.let { return it }
                worktrees.firstOrNull { (it.worktree ?: "").trim('/').let { p -> p == path || p.endsWith("/$path") } }?.let { return it }
            }
            return if (lane.branch.isEmpty()) null else worktrees.firstOrNull { it.branch == lane.branch }
        }

        fun build(): Tree {
            val work = themeNodes() + listOfNotNull(noThemeNode(), unreadableNode())
            val reached = mutableSetOf<String>()
            fun walk(nodes: List<Node>) {
                nodes.forEach { n ->
                    n.worktreeId?.let(reached::add)
                    (n.target as? Target.Worktree)?.let { reached += it.id }
                    walk(n.children)
                }
            }
            walk(work)
            return Tree(work, listOfNotNull(mainCheckoutNode(), looseNode(reached)))
        }

        /** Cards in a status this build doesn't know: said, in a group of their own, rather than dropped. */
        fun unreadableNode(): Node? {
            if (unreadable.isEmpty()) return null
            val id = "group:unreadable"
            return Node(id, Kind.GROUP, Words.NOT_ON_THIS_VERSION, detail = "${unreadable.size}", caption = Words.NOT_ON_THIS_VERSION_CAPTION,
                children = unreadable.map { Node("$id/card:${it.id}", Kind.TASK, it.title, key = it.key, detail = it.status, quiet = true) })
        }

        fun lanes(task: String) = lanesByTask[task].orEmpty()

        fun themeNodes(): List<Node> = themes.mapNotNull { theme ->
            val ids = theme.cards.map { it.task }
            val shown = ids.mapNotNull { cards[it] }.filter { filter.shows(it.status) }
            if (filter == Filter.OPEN && theme.state == "done" && shown.isEmpty()) return@mapNotNull null
            if (filter == Filter.IN_REVIEW && shown.isEmpty()) return@mapNotNull null
            val id = "theme:${theme.id}"
            val children = shown.map { taskNode(it, id) }.toMutableList()
            if (filter == Filter.OPEN) {
                val done = ids.mapNotNull { cards[it] }.filter { it.status == TaskStatus.DONE }
                if (done.isNotEmpty()) {
                    children += Node("$id/done", Kind.DONE_FOLD, Words.done(done.size), children = done.map { taskNode(it, id) }, quiet = true)
                }
            }
            children += pages.filter { it.themeAnchor == theme.id }.map { Node("$id/page:${it.slot}", Kind.PAGE, it.title, target = Target.Page(it.slot)) }
            Node(id, Kind.THEME, theme.name, detail = Words.progress(theme.counts), target = Target.Theme(theme.id), children = children, asks = theme.id in askThemes)
        }

        fun noThemeNode(): Node? {
            val themed = themes.flatMap { t -> t.cards.map { it.task } }.toSet()
            val order = TaskStatus.ORDER.withIndex().associate { it.value to it.index }
            val mine = rows.filter { it.id !in themed && filter.shows(it.status) }
                .sortedWith(compareBy<TaskRow> { order[it.status] ?: 99 }.thenByDescending { it.lastMovedMs })
                .mapNotNull { cards[it.id] }
            val done = if (filter == Filter.OPEN) rows.filter { it.id !in themed && it.status == TaskStatus.DONE }.mapNotNull { cards[it.id] } else emptyList()
            if (mine.isEmpty() && done.isEmpty()) return null
            val id = "group:no-theme"
            val children = mine.map { taskNode(it, id) }.toMutableList()
            if (done.isNotEmpty()) children += Node("$id/done", Kind.DONE_FOLD, Words.done(done.size), children = done.map { taskNode(it, id) }, quiet = true)
            return Node(id, Kind.GROUP, Words.NO_THEME, detail = "${mine.size}", children = children)
        }

        fun taskNode(card: Card, parent: String): Node {
            val id = "$parent/task:${card.id}"
            val mine = lanes(card.id)
            val children = mine.map { laneNode(it, card, id) }.toMutableList()
            children += ownWorktrees(card).map { wt ->
                val wid = "$id/worktree:${wt.id}"
                Node(wid, Kind.LANE, wt.task, target = Target.Worktree(wt.id), children = terminalNodes(wt, wid), worktreeId = wt.id)
            }
            if (mine.isEmpty()) children += workerNodes(card.workers, id)
            return Node(id, Kind.TASK, card.title, key = card.key, target = Target.Task(card.id), children = children, asks = card.id in askTasks, quiet = card.status.isFinished)
        }

        fun ownWorktrees(card: Card): List<Worktree> {
            val taken = lanes(card.id).mapNotNull { laneWorktrees[it.id]?.id }.toSet()
            val out = mutableListOf<Worktree>()
            card.worktreeId?.let { own -> worktrees.firstOrNull { it.id == own && it.id !in taken }?.let(out::add) }
            worktrees.filter { card.id in taskIds(it) && it.id !in taken && out.none { o -> o.id == it.id } }.forEach(out::add)
            return out
        }

        fun laneNode(lane: PlanLane, card: Card, parent: String): Node {
            val id = "$parent/lane:${lane.id}"
            val wt = laneWorktrees[lane.id]
            val children = (wt?.let { terminalNodes(it, id) } ?: emptyList()) + subagents(lane, id)
            return Node(
                id, Kind.LANE, lane.name, detail = PlanWords.state(lane.state),
                also = Words.also(lane.cards.filter { it.task != card.id }.map { it.key }),
                target = Target.Lane(lane.id), children = children, quiet = !lane.state.isLive, worktreeId = wt?.id,
            )
        }

        fun terminalNodes(wt: Worktree, parent: String): List<Node> =
            wt.terminals.filter { !it.isOrchestrator && !it.isChangesPane }.map { t ->
                val agent = t.isAgentPane || t.agent != AgentActivity.NONE
                Node("$parent/terminal:${t.id}", Kind.TERMINAL, t.label, detail = if (agent) "Agent" else "",
                    target = Target.Terminal(wt.id, t.id), asks = t.id in askTerminals, worktreeId = wt.id)
            }

        /** A live lane's open agents; where it records none, its cards' open subagents on their newest live lane. */
        fun subagents(lane: PlanLane, parent: String): List<Node> {
            if (!lane.state.isLive) return emptyList()
            val open = lane.agents.filter { it.endedAt == null }
            if (open.isNotEmpty()) return open.mapIndexed { i, a ->
                subagent("$parent/agent:${a.agentId.ifEmpty { "$i" }}", Words.subagent(Words.role(a.role), a.model))
            }
            val workers = lane.cards.flatMap { c ->
                if (lanes(c.task).lastOrNull { it.state.isLive }?.id != lane.id) emptyList() else cards[c.task]?.workers.orEmpty()
            }
            return workerNodes(workers, parent)
        }

        fun workerNodes(workers: List<TaskWorker>, parent: String) = workers.filter { it.state.isOpen }.mapIndexed { i, w ->
            subagent("$parent/worker:$i", Words.subagent("Subagent", w.model.ifEmpty { w.harness }))
        }

        fun subagent(id: String, title: String) = Node(id, Kind.SUBAGENT, title, caption = Words.SUBAGENT_CAPTION, target = Target.Orchestrator)

        fun mainCheckoutNode(): Node? {
            val wt = checkout ?: return null
            val id = "group:main"
            val terminals = wt.terminals.filter { !it.isOrchestrator && it.taskId == null && !it.isChangesPane }.map { t ->
                Node("$id/terminal:${t.id}", Kind.TERMINAL, t.label, target = Target.Terminal(wt.id, t.id), worktreeId = wt.id)
            }
            return Node(id, Kind.GROUP, Words.MAIN_CHECKOUT, detail = Words.shells(terminals.size), target = Target.Worktree(wt.id), children = terminals, worktreeId = wt.id)
        }

        /**
         * Loose worktrees: the workspace's worktrees no row the tree shows
         * reaches, a cancelled card's under Not done among them; the hidden
         * ones in a closed group of their own.
         */
        fun looseNode(reached: Set<String>): Node? {
            val unreached = worktrees.filter { it.id !in reached }
            if (unreached.isEmpty()) return null
            val id = "group:loose"
            fun row(wt: Worktree, parent: String): Node {
                val wid = "$parent/worktree:${wt.id}"
                return Node(wid, Kind.WORKTREE, wt.task, target = Target.Worktree(wt.id), children = terminalNodes(wt, wid), quiet = wt.isHidden, worktreeId = wt.id)
            }
            val shown = unreached.filter { !it.isHidden }
            val hidden = unreached.filter { it.isHidden }
            val children = shown.map { row(it, id) } + listOfNotNull(
                if (hidden.isEmpty()) null
                else Node("$id/hidden", Kind.GROUP, Words.HIDDEN, detail = "${hidden.size}", children = hidden.map { row(it, "$id/hidden") }, quiet = true),
            )
            return Node(id, Kind.GROUP, Words.LOOSE_WORKTREES, detail = "${shown.size}", children = children)
        }
    }
}
