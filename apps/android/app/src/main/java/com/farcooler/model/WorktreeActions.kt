package com.farcooler.model

/**
 * What a worktree's menu offers, wherever its row is drawn: the Worktrees
 * list's header and the One tree's lane, loose-worktree and checkout rows
 * (ov-300). One rule, so the tree that took the Worktrees tab's place can't
 * offer less than the tab did.
 */
enum class WorktreeAction(val title: String) {
    NEW_TERMINAL("New terminal"),
    STACK("Stack and pull request"),
    MOVE_UP("Move up"),
    MOVE_DOWN("Move down"),
    HIDE("Hide"),
    UNHIDE("Unhide"),
    REMOVE("Remove worktree"),
}

object WorktreeActions {
    /**
     * [worktree]'s actions, in menu order. Stack only where the runner said
     * which repository and branch it's on. Move up and down only where the
     * runner keeps an order (`ordinal`) and there is a row to pass ([above],
     * [below]: the neighbors in the list it's drawn in, by id). Remove never
     * for the repository's own checkout, which the runner refuses anyway.
     */
    fun of(worktree: Worktree, above: String? = null, below: String? = null): List<WorktreeAction> = buildList {
        add(WorktreeAction.NEW_TERMINAL)
        if (worktree.repository != null && worktree.branch.isNotBlank()) add(WorktreeAction.STACK)
        if (worktree.ordinal != null) {
            if (above != null) add(WorktreeAction.MOVE_UP)
            if (below != null) add(WorktreeAction.MOVE_DOWN)
        }
        add(if (worktree.isHidden) WorktreeAction.UNHIDE else WorktreeAction.HIDE)
        if (!worktree.isMainCheckout) add(WorktreeAction.REMOVE)
    }

    /**
     * The runner's order with [id] moved past its neighbor [beside]: above it
     * going [up], else below it. [order] unchanged when either isn't in it.
     */
    fun moved(order: List<String>, id: String, beside: String, up: Boolean): List<String> =
        WorktreeOrder.moved(order, id, beside, if (up) WorktreeOrder.Edge.ABOVE else WorktreeOrder.Edge.BELOW)
}
