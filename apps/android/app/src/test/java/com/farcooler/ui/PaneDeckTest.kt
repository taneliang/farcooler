package com.farcooler.ui

import com.farcooler.model.Terminal
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Which of a workspace's tabs are alive, and which one goes when there are too
 * many.
 *
 * The half of the pane-lifetime work that a JVM can answer. What is pinned here
 * is identity and bookkeeping: that a tab you come back to is the SAME tab, that
 * the mount limit evicts the one shown longest ago, and that a poll which
 * briefly answers nothing does not take every mounted pane with it.
 *
 * What is NOT here, and cannot be without hardware: that a scroll position
 * actually survives on screen, that a hidden pane really stops streaming, and
 * that a mounted pane's `remember` is preserved by Compose across a change in
 * the mounted set. The first two need a runner and a device; the third is
 * `key(...)`'s own contract, and testing it would be testing Compose.
 */
class PaneDeckTest {
    private fun tab(id: String) = Pane.Terminal(id)

    private fun terminal(id: String, paneMode: String? = null) =
        Terminal(id = id, paneMode = paneMode)

    private fun terminals(vararg ids: String) = ids.map { terminal(it) }

    /** The invariant the two lists exist under, checked after every move. */
    private fun assertConsistent(deck: PaneDeck) {
        assertTrue("mounted and recent disagree: $deck", deck.sameSet)
        assertTrue("the current tab is not mounted: $deck", deck.isMounted(deck.current))
        assertEquals("the current tab is not the most recent: $deck", deck.current, deck.recent.last())
        assertTrue("over the limit: $deck", deck.mountedPanes <= PaneDeck.MOUNT_LIMIT)
    }

    // ---- Identity: a tab you come back to is the same tab ----

    /**
     * The whole point, stated as bookkeeping.
     *
     * Coming back to a tab must not MOUNT it again — mounting is what builds a
     * new `TerminalSession` and a new `AgentStream`, and that is F-3: the tab
     * you returned to was one that had never been open.
     */
    @Test
    fun comingBackToATabDoesNotMountItAgain() {
        var deck = PaneDeck.opening(tab("a"))
        deck = deck.select(tab("b"))
        val afterFirstVisit = deck.mounted
        deck = deck.select(tab("a"))

        assertEquals(afterFirstVisit, deck.mounted)
        assertEquals(tab("a"), deck.current)
        assertConsistent(deck)
    }

    /**
     * The mounted order never moves, and that is deliberate.
     *
     * It is the order the composable walks. A reorder moves composition groups
     * and the layout nodes beneath them — including the `AndroidView` the
     * software keyboard is attached to — and there is no reason to make `key`
     * prove it can survive that on every single tab tap.
     */
    @Test
    fun theMountedOrderIsTheOrderTabsWereFirstOpened() {
        var deck = PaneDeck.opening(tab("a"))
        deck = deck.select(tab("b"))
        deck = deck.select(tab("a"))
        deck = deck.select(tab("b"))

        assertEquals(listOf(tab("a"), tab("b")), deck.mounted)
        assertEquals(listOf(tab("a"), tab("b")), deck.recent)
    }

    @Test
    fun selectingTheTabAlreadyShowingChangesNothing() {
        val deck = PaneDeck.opening(tab("a")).select(tab("b"))
        assertEquals(deck, deck.select(tab("b")))
    }

    @Test
    fun changesIsATabLikeAnyOther() {
        var deck = PaneDeck.opening(tab("a"))
        deck = deck.select(Pane.Changes)
        assertEquals(Pane.Changes, deck.current)
        assertTrue(deck.isMounted(tab("a")))
        assertConsistent(deck)
    }

    /**
     * Opening the diff must not cost an agent its session.
     *
     * The limit counts emulators, streams and native scrollback, and the
     * Changes tab has none of the three. Spending a slot on it would evict a
     * conversation to buy nothing.
     */
    @Test
    fun theChangesTabIsFreeAndIsNeverEvicted() {
        var deck = PaneDeck.opening(Pane.Changes)
        for (i in 1..PaneDeck.MOUNT_LIMIT) deck = deck.select(tab("t$i"))

        assertTrue(deck.isMounted(Pane.Changes))
        assertEquals(PaneDeck.MOUNT_LIMIT, deck.mountedPanes)
        assertEquals(PaneDeck.MOUNT_LIMIT + 1, deck.mounted.size)
        assertConsistent(deck)

        // And one more agent still evicts an agent, not the diff.
        deck = deck.select(tab("one-more"))
        assertTrue(deck.isMounted(Pane.Changes))
        assertFalse(deck.isMounted(tab("t1")))
        assertConsistent(deck)
    }

    // ---- The limit ----

    /**
     * The one shown longest ago goes, not the one mounted longest ago.
     *
     * The two only differ when somebody revisits, and when they do,
     * first-mounted evicts the tab they just came back to — which is the worst
     * answer available. So `t0` is re-shown just before the deck overflows,
     * which leaves `t1` as the one that has been sitting unread the longest.
     *
     * **The tab count is derived from the limit and that is not tidiness.** This
     * was written with four literal tabs against a limit that happened to be
     * three, and when the limit went to five it stopped overflowing at all: the
     * test still ran, still asserted, and was no longer about eviction. It
     * failed loudly — `expected:<5> but was:<4>` — only because it also pinned
     * `mountedPanes` to the constant. A version that had checked eviction alone
     * would have gone quietly green while testing nothing, which is this repo's
     * defining failure mode wearing a passing badge.
     */
    @Test
    fun theTabShownLongestAgoIsTheOneEvicted() {
        // Fill the deck exactly: t0 through t(limit - 1).
        var deck = PaneDeck.opening(tab("t0"))
        for (i in 1 until PaneDeck.MOUNT_LIMIT) deck = deck.select(tab("t$i"))
        assertEquals("the deck should be full and not yet over", PaneDeck.MOUNT_LIMIT, deck.mountedPanes)

        // Come back to the oldest, which makes the SECOND-oldest the least
        // recently shown. This is the step that separates least-recently-shown
        // from first-mounted; without it both rules would evict `t0`.
        deck = deck.select(tab("t0"))

        // One more, which must now overflow.
        deck = deck.select(tab("overflow"))

        assertEquals(PaneDeck.MOUNT_LIMIT, deck.mountedPanes)
        assertFalse("the least recently shown goes", deck.isMounted(tab("t1")))
        assertTrue("the one just revisited stays", deck.isMounted(tab("t0")))
        assertTrue(deck.isMounted(tab("t${PaneDeck.MOUNT_LIMIT - 1}")))
        assertEquals(tab("overflow"), deck.current)
        assertConsistent(deck)
    }

    /**
     * **The limit is five, and five is three plus two.**
     *
     * Every other test in this file is written against [PaneDeck.MOUNT_LIMIT]
     * rather than a number, which is right — they are about the RULE and should
     * survive the constant moving. But it means they would all pass at a limit
     * of one, so nothing here pinned the value itself, and the value is a
     * decision with an argument behind it:
     *
     *   - THREE is what "the agents I am working with" means in one worktree,
     *     the same answer `AGENTS_PER_WORKSPACE` reached independently.
     *   - TWO more are the shell's track, which draws the pane you are on with
     *     a real neighbour either side — that is what makes an incoming
     *     terminal a terminal rather than a placeholder that appears on commit
     *     — and either neighbour can sit in another workspace.
     *
     * If this fails, the limit moved. That is allowed, and `PaneDeck`'s comment
     * says what evidence should move it (`REASON_LOW_MEMORY` in `ProcessExit`'s
     * log); this test is here so it cannot move by accident, and so that the
     * next person reads the argument before changing the number.
     */
    @Test
    fun theLimitIsThreeAgentsPlusTheTrackTwoNeighbours() {
        assertEquals(3 + 2, PaneDeck.MOUNT_LIMIT)
    }

    /**
     * A track's worth of panes fits without evicting anything.
     *
     * The arithmetic above, exercised rather than asserted: the pane you are on,
     * two more you have been working with, and the two neighbours the track
     * mounts either side of you. If the limit ever drops below this, a swipe
     * starts destroying a pane to draw its neighbour — which is the one thing
     * this whole type exists to prevent.
     */
    @Test
    fun aTrackWithBothNeighboursEvictsNothing() {
        var deck = PaneDeck.opening(tab("current"))
        for (name in listOf("second", "third", "previous", "next")) {
            deck = deck.select(tab(name))
        }
        for (name in listOf("current", "second", "third", "previous", "next")) {
            assertTrue("$name should still be mounted", deck.isMounted(tab(name)))
        }
        assertEquals(5, deck.mountedPanes)
        assertConsistent(deck)
    }

    /** Whatever the limit is, the tab being shown can never be the one dropped. */
    @Test
    fun theTabBeingShownIsNeverEvicted() {
        var deck = PaneDeck.opening(tab("t0"))
        for (i in 1..12) {
            deck = deck.select(tab("t$i"))
            assertEquals(tab("t$i"), deck.current)
            assertConsistent(deck)
        }
    }

    // ---- Pruning ----

    @Test
    fun aPaneTheRunnerNoLongerHasIsUnmounted() {
        var deck = PaneDeck.opening(tab("a")).select(tab("b"))
        deck = deck.prune(terminals("a"))

        assertEquals(listOf(tab("a")), deck.mounted)
        assertEquals(tab("a"), deck.current)
        assertConsistent(deck)
    }

    /**
     * Where you were was taken away, so you land on the closest thing to it.
     *
     * The most recently shown survivor, and never a rule's opinion about which
     * agent matters: being moved because the runner took something away is not
     * a preference, and it must not be recorded as one either — which is why
     * this returns a deck rather than writing a focus.
     */
    @Test
    fun losingTheTabYouWereOnLandsOnTheLastOneYouRead() {
        var deck = PaneDeck.opening(tab("a"))
        deck = deck.select(tab("b"))
        deck = deck.select(tab("c"))
        deck = deck.prune(terminals("a", "b"))

        assertEquals(tab("b"), deck.current)
        assertConsistent(deck)
    }

    /**
     * A poll that briefly answers nothing must not take the workspace with it.
     *
     * A reconnect or a runner mid-restart returns an empty fleet for a moment,
     * and unmounting every pane on that evidence would throw away exactly the
     * state this type exists to keep. A runner that is genuinely gone is handled
     * a level up, where the whole route comes off the stack.
     */
    @Test
    fun anEmptyAnswerPrunesNothing() {
        val deck = PaneDeck.opening(tab("a")).select(tab("b"))
        assertEquals(deck, deck.prune(emptyList()))
    }

    /** Changes is the floor: a worktree with no panes left still has a diff. */
    @Test
    fun aWorkspaceThatLosesEveryPaneFallsToChanges() {
        var deck = PaneDeck.opening(tab("a")).select(tab("b"))
        deck = deck.prune(terminals("somebody-elses-pane"))

        assertEquals(Pane.Changes, deck.current)
        assertEquals(listOf(Pane.Changes), deck.mounted)
        assertConsistent(deck)
    }

    /**
     * A pane switched to `changes` mode from the Mac stops being its own tab.
     *
     * Not because it is gone — it is still in the fleet — but because it has
     * become the Changes tab, and two tabs onto one diff is a choice with no
     * difference behind it.
     */
    @Test
    fun aPaneThatBecomesAChangesPaneFoldsIntoTheChangesTab() {
        var deck = PaneDeck.opening(tab("a"))
        deck = deck.prune(listOf(terminal("a", paneMode = "changes")))

        assertEquals(Pane.Changes, deck.current)
        assertFalse(deck.isMounted(tab("a")))
        assertConsistent(deck)
    }

    @Test
    fun pruningLeavesADeckAloneWhenNothingWent() {
        val deck = PaneDeck.opening(tab("a")).select(tab("b"))
        assertEquals(deck, deck.prune(terminals("a", "b", "c")))
    }

    // ---- Identity, which is what the draft-per-pane fix rests on ----

    /**
     * Two tabs can never share an id, and that is what keeps two drafts apart.
     *
     * `c37f487` recorded that the composer's draft was keyed to nothing and so
     * was shared across every pane in the fleet, and said it was waiting on this
     * phase. The fix is that the composable is per pane and that each pane's
     * `rememberSaveable` state is bucketed under this string by the workspace
     * screen's `SaveableStateHolder`. So the whole guarantee reduces to these
     * ids being distinct — including against the one string that is not a
     * terminal id at all, which is exactly where a collision would be silent.
     */
    @Test
    fun everyTabHasItsOwnIdentity() {
        val ids = listOf(tab("a"), tab("b"), Pane.Changes).map { it.id }
        assertEquals(ids.size, ids.toSet().size)

        // The namespace is what makes that true for a terminal a daemon
        // happened to name "changes".
        assertTrue(tab("changes").id != Pane.Changes.id)
        assertEquals(tab("changes"), Pane.parse(tab("changes").id))
        assertEquals(Pane.Changes, Pane.parse(Pane.Changes.id))
    }
}
