package com.farcooler.ui

import com.farcooler.model.Terminal

/**
 * Which of a workspace's tabs are mounted, which one is on screen, and which
 * one goes when there are more than this phone should hold.
 *
 * The bookkeeping behind the mounted-pane discipline, kept out of the
 * composable so it can be tested without a device — the same split
 * `model/NeedsYou.kt` and [Backstack] make. Everything here is pure: a deck in,
 * a deck out, no session and no fleet beyond the list of terminals that still
 * exist.
 *
 * ## Why anything is mounted at all
 *
 * The screen now split into `ui/WorkspaceScreen.kt` and `ui/TerminalPane.kt`
 * built ONE [com.farcooler.net.TerminalSession] and
 * re-pointed it with `switchTo` on every tab tap, and `ui/AgentScreen.kt` built
 * a fresh [com.farcooler.net.AgentStream] per terminal id. So switching tabs
 * threw away the other tab's scroll position, its half-typed message and its
 * open stream, and coming back rebuilt all three — which is a "Loading…" on a
 * pane you had already opened, a transcript that starts at the top, and a
 * terminal renegotiating its size with tmux, so the content jumps. F-3 in the
 * parity inventory, and the loss iOS's `WorkspaceView` exists to prevent.
 *
 * Mounted panes are not destroyed, so there is no scroll position to restore,
 * no draft to save and reload, and no code that can get either wrong. A whole
 * class of bug stops being representable.
 *
 * ## What is NOT kept by mounting
 *
 * Anything that costs the runner. A hidden pane holds no SSH stream, runs no
 * agent poll and asserts no tmux geometry — see `TerminalSession.stop` and
 * `resume`. Mounting keeps the STATE; visibility keeps the TRAFFIC, and
 * conflating them would mean a phone with four tabs open holding four SSH
 * channels while it sits in a pocket.
 *
 * ## Why there is a limit, where iOS has none
 *
 * `WorkspaceView` keeps every visited pane for the life of the screen. It can
 * afford to: a mounted-but-hidden pane on iOS is a view and an emulator, on a
 * device with a memory ceiling nobody is fighting for.
 *
 * A mounted terminal pane here holds a `VtCore` — a native alacritty grid
 * bounded by `SCROLLBACK_LINES = 10_000` in `crates/vt/src/lib.rs`, filled from
 * the daemon's replay of the whole of tmux's history — plus a dedicated
 * emulator thread. That is native RSS, which is exactly what Android's
 * low-memory killer counts, and it is not the JVM heap anybody is watching. A
 * workspace with eight `claude` panes visited in one sitting would hold eight
 * of them.
 *
 * So [MOUNT_LIMIT] panes, and the one evicted is the one shown longest ago.
 * Least-recently-shown rather than first-mounted, because the two only differ
 * when somebody revisits — and when they do, first-mounted evicts the tab they
 * just came back to, which is the worst possible answer.
 *
 * **What an evicted pane loses, precisely.** Its `TerminalSession` is disposed:
 * the emulator's own scrollback and the scroll position within it go, and its
 * `AgentStream`'s in-memory transcript rows and their scroll offset go. What
 * comes back on the next visit is not a blank pane — the daemon replays tmux's
 * history for a terminal, and `terminal.agent_subscribe` replays the transcript
 * from the cursor the new stream starts at — so what is actually lost is *where
 * you were*, not what was there. And the half-typed message survives even that,
 * because the workspace screen buckets each pane's saveable state under
 * [Pane.id] in a `SaveableStateHolder`, which deliberately keeps the state of
 * keys that have left the composition. Nothing here has to know that; it is
 * written down because it is what makes a cap bearable.
 */
data class PaneDeck(
    /** The tab on screen. Always in [mounted], and always last in [recent]. */
    val current: Pane,
    /**
     * Every mounted tab, in the order it was first shown.
     *
     * **Never reordered**, which is not tidiness: this is the order the
     * composable walks, and a reorder moves composition groups and the layout
     * nodes under them — including the `AndroidView` the software keyboard is
     * attached to. `key(...)` would preserve state across the move, but there
     * is no reason to make it prove that on every tab tap when a stable order
     * and a z-index say the same thing for free.
     */
    val mounted: List<Pane>,
    /**
     * The same set, least recently shown FIRST. Eviction takes the head.
     *
     * A second list rather than a timestamp per pane, because "what goes next"
     * is then a value a test can read rather than a comparison it has to
     * reproduce. The two lists hold the same set at all times — [sameSet] is
     * that invariant, asserted rather than assumed.
     */
    val recent: List<Pane>,
) {
    /** Whether [pane] is drawn at all, as opposed to being the one on screen. */
    fun isMounted(pane: Pane): Boolean = pane in mounted

    /** What the limit is actually counting. See [MOUNT_LIMIT]. */
    val mountedPanes: Int get() = mounted.count { it is Pane.Terminal }

    /** The two lists agree about which panes exist. Pinned by `PaneDeckTest`. */
    val sameSet: Boolean get() = mounted.toSet() == recent.toSet() && mounted.size == recent.size

    /**
     * Show a tab, mounting it the first time it is asked for.
     *
     * Idempotent on the tab already showing, deliberately. Every road into a
     * selection funnels here — the strip, a notification tap, a fleet row, the
     * drawer — and one of them arriving twice about the same pane has to be
     * free rather than a rebuild.
     */
    fun select(pane: Pane): PaneDeck {
        // Moved to the end whether or not it was already there: being shown is
        // what recency means, and a tab you keep coming back to must not age
        // out behind one you opened once.
        val bumped = recent.filterNot { it == pane } + pane
        if (pane in mounted) return PaneDeck(pane, mounted, bumped)

        var nowMounted = mounted + pane
        var nowRecent = bumped
        while (nowMounted.count { it is Pane.Terminal } > MOUNT_LIMIT) {
            // The least recently shown TERMINAL. Never the tab being shown: it
            // was just moved to the end of recency, and the limit is at least
            // one. Never the Changes tab either — see [MOUNT_LIMIT].
            val evicted = nowRecent.first { it is Pane.Terminal }
            nowMounted = nowMounted.filterNot { it == evicted }
            nowRecent = nowRecent.filterNot { it == evicted }
        }
        return PaneDeck(pane, nowMounted, nowRecent)
    }

    /**
     * Drop tabs for panes the runner no longer has.
     *
     * A pane that no longer exists cannot be the one on screen, and a mounted
     * one would sit there forever holding a session for a pane the host has
     * forgotten — which on this platform is a native emulator handle and a
     * thread, not merely a stale view.
     *
     * **Never prunes to nothing.** A poll that briefly returns an empty
     * workspace — a reconnect, a runner mid-restart — would otherwise unmount
     * every pane and throw away exactly the state this type exists to keep. A
     * runner that is genuinely gone is handled a level up, where `AppModel`
     * takes the whole route off the stack.
     *
     * **Changes is the floor.** A worktree whose last agent was stopped while
     * you were reading it still has a diff, which is usually why you were
     * there; the alternative is a screen with nothing on it.
     *
     * A terminal that has since been switched to `changes` mode is pruned too,
     * for a different reason: it is not gone, it has become the Changes tab,
     * and leaving both would be two tabs onto one diff. See [Pane].
     */
    fun prune(live: List<Terminal>): PaneDeck {
        if (live.isEmpty()) return this
        val stillAPane = live.filterNot { it.isChangesPane }.map { it.id }.toSet()
        fun keep(pane: Pane) = pane !is Pane.Terminal || pane.terminalId in stillAPane

        val nowMounted = mounted.filter(::keep)
        if (nowMounted.size == mounted.size) return this
        val nowRecent = recent.filter(::keep)

        if (keep(current)) {
            // Nothing that was on screen went, so nothing moves. Only the
            // hidden tabs were trimmed.
            return PaneDeck(current, nowMounted, nowRecent)
        }
        // Where you were was taken away. The most recently shown survivor is
        // the closest thing to where you meant to be — and Changes underneath
        // it when there is no survivor at all.
        val next = nowRecent.lastOrNull() ?: Pane.Changes
        val mountedWithNext = if (next in nowMounted) nowMounted else nowMounted + next
        val recentWithNext = if (next in nowRecent) nowRecent else nowRecent + next
        return PaneDeck(next, mountedWithNext, recentWithNext)
    }

    companion object {
        /**
         * How many of a workspace's PANES stay mounted.
         *
         * Three, which is what "the agents I am working with" means in one
         * worktree. `AGENTS_PER_WORKSPACE` in `model/NeedsYou.kt` independently
         * arrived at three for the front door, and the agreement is not a
         * coincidence — both are asking how many agents of one worktree a
         * person holds in their head at once. It is deliberately NOT that
         * constant: one is a memory budget and the other is a row budget, and a
         * shared name would tie a phone's RSS to a list's height.
         *
         * **The Changes tab does not count against it and is never evicted.**
         * This is a budget for emulators, streams and native scrollback, and
         * that tab has none of the three — it is asked for by workspace id and
         * holds nothing on the runner. Spending a slot on it would evict an
         * agent to buy nothing. Worth re-reading when phase 5 gives it a diff
         * to hold: the reason is about what the tab costs, so a tab that starts
         * costing something is a reason to revisit this, not to widen it
         * quietly.
         */
        const val MOUNT_LIMIT = 3

        /** A workspace opening on one tab, with nothing else mounted yet. */
        fun opening(pane: Pane) = PaneDeck(pane, listOf(pane), listOf(pane))
    }
}
