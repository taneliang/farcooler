package com.farcooler.ui

import com.farcooler.model.Terminal
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Where the app is, written down and read back.
 *
 * Two things are being pinned here and they fail in different ways.
 *
 * The **encoding** is a wire format for a phone's own state: it is written by
 * one build of the app and read by whichever build is installed when the
 * process comes back. A route renamed on one side of that gap costs somebody
 * their place silently — no crash, no log line, just the workspace list where
 * the diff they were reading used to be. So the discriminators are asserted
 * literally rather than round-tripped, because a round trip passes happily
 * while both ends rename together.
 *
 * The **resolve-or-truncate rule** is what stands between a restored route and
 * a broken screen. It is pure by construction — [Backstack] takes the liveness
 * question as a lambda precisely so this can ask it without a runner, a socket
 * or a device.
 *
 * What is NOT here, and cannot be: that a real process death restores, and that
 * the back gesture animates. Both need hardware.
 */
class BackstackTest {
    private fun terminal(id: String, state: String = "exited", activity: String? = null) =
        Terminal(id = id, state = state, activity = activity)

    // ---- The encoding ----

    @Test
    fun everyRouteSurvivesTheRoundTrip() {
        val stack = listOf(
            Route.Fleet,
            Route.Terminal("host-a", "workspace-1"),
            Route.Settings,
            Route.RunnerSettings("host-b"),
        )
        assertEquals(stack, Backstack.decodeStack(Backstack.encodeStack(stack)))

        // The cases with no fields are the ones a hierarchy change is most
        // likely to break quietly, so each is named rather than sampled.
        for (route in listOf(
            Route.Onboarding,
            Route.Fleet,
            Route.Settings,
            Route.Authorize,
            Route.Join,
            Route.AddDevice,
            Route.Devices,
        )) {
            assertEquals(listOf(route), Backstack.decodeStack(Backstack.encodeStack(listOf(route))))
        }
    }

    @Test
    fun theStackKeepsItsOrder() {
        val stack = listOf(Route.Terminal("h", "w"), Route.Settings, Route.RunnerSettings("h"))
        val back = Backstack.decodeStack(Backstack.encodeStack(stack))!!
        assertEquals(Route.RunnerSettings("h"), back.last())
        assertEquals(Route.Terminal("h", "w"), back.first())
    }

    /**
     * The discriminators, literally.
     *
     * These strings are the format, not an implementation detail: renaming a
     * route case or moving [Route] to another package would change them and
     * strand every saved stack. If this test has to be edited, the edit is a
     * decision to drop everybody's restored place, not a rename.
     */
    @Test
    fun theWireNamesAreTheOnesOnDisk() {
        assertEquals(
            """[{"type":"fleet"},{"type":"terminal","hostId":"h","workspaceId":"w"}]""",
            Backstack.encodeStack(listOf(Route.Fleet, Route.Terminal("h", "w"))),
        )
        assertEquals(
            """[{"type":"runner-settings","hostId":"h"}]""",
            Backstack.encodeStack(listOf(Route.RunnerSettings("h"))),
        )
        assertEquals(
            """[{"type":"onboarding"},{"type":"authorize"},{"type":"join"}]""",
            Backstack.encodeStack(listOf(Route.Onboarding, Route.Authorize, Route.Join)),
        )
        assertEquals(
            """[{"type":"add-device"},{"type":"devices"},{"type":"settings"}]""",
            Backstack.encodeStack(listOf(Route.AddDevice, Route.Devices, Route.Settings)),
        )
    }

    /**
     * The runner is IN the route, and two runners' routes are different routes.
     *
     * Android connects to every runner at once where iOS makes you pick one, so
     * a workspace id alone is ambiguous here in a way it is not there: ids are
     * minted per daemon. A stack that lost the host would restore onto whatever
     * runner happened to answer first.
     */
    @Test
    fun aRestoredRouteStillKnowsWhichRunnerItIsOn() {
        val stack = listOf(Route.Terminal("host-a", "shared-id"), Route.Terminal("host-b", "shared-id"))
        val back = Backstack.decodeStack(Backstack.encodeStack(stack))!!
        assertEquals("host-a", (back[0] as Route.Terminal).hostId)
        assertEquals("host-b", (back[1] as Route.Terminal).hostId)
        assertTrue(back[0] != back[1])
    }

    @Test
    fun nothingUsableDecodesToNothing() {
        assertNull(Backstack.decodeStack(null))
        assertNull(Backstack.decodeStack(""))
        assertNull(Backstack.decodeStack("   "))
        assertNull(Backstack.decodeStack("not json at all"))
        // An empty stack is not a place. There is no such thing as being
        // nowhere, so this has to read as "nothing was saved".
        assertNull(Backstack.decodeStack("[]"))
    }

    /**
     * A route type this build has never heard of.
     *
     * What a phone sees when it is downgraded, or when a beta wrote a stack a
     * stable build then reads. The whole stack goes rather than the routes
     * before the unknown one, because recovering a prefix would be guessing at
     * what somebody meant from a format this build does not speak.
     */
    @Test
    fun anUnknownRouteCostsTheWholeStackAndNotTheApp() {
        assertNull(
            Backstack.decodeStack("""[{"type":"fleet"},{"type":"changes","workspaceId":"w"}]""")
        )
    }

    /** A field added by a later build must not cost an older one its place. */
    @Test
    fun anUnknownFieldIsIgnored() {
        assertEquals(
            listOf(Route.Terminal("h", "w")),
            Backstack.decodeStack(
                """[{"type":"terminal","hostId":"h","workspaceId":"w","scrollTo":"line-40"}]"""
            ),
        )
    }

    // ---- The focus memory ----

    /**
     * Only a choice is written down.
     *
     * A fleet-list row and a tapped notification are where somebody was SENT.
     * Persisting those would make the memory a record of the app's own guesses,
     * read back tomorrow in preference to the rule that would make the guess
     * again with tomorrow's fleet in front of it — so a 3am ping would decide
     * where that workspace opens for good.
     */
    @Test
    fun onlyAChosenTabIsRememberedAcrossTheProcess() {
        val live = mapOf(
            "host-a/w1" to Focus("t-chosen", chosen = true),
            "host-a/w2" to Focus("t-sent", chosen = false),
        )
        val encoded = Backstack.encodeFocus(live)
        assertEquals("""{"host-a/w1":"t-chosen"}""", encoded)

        val back = Backstack.decodeFocus(encoded)
        assertEquals(mapOf("host-a/w1" to Focus("t-chosen", chosen = true)), back)
    }

    @Test
    fun aFocusIsKeyedByRunnerAsWellAsWorkspace() {
        val live = mapOf(
            Backstack.key("host-a", "shared-id") to Focus("t1", chosen = true),
            Backstack.key("host-b", "shared-id") to Focus("t2", chosen = true),
        )
        val back = Backstack.decodeFocus(Backstack.encodeFocus(live))
        assertEquals(2, back.size)
        assertEquals("t1", back[Backstack.key("host-a", "shared-id")]?.terminalId)
        assertEquals("t2", back[Backstack.key("host-b", "shared-id")]?.terminalId)
    }

    @Test
    fun nothingUsableDecodesToAnEmptyMemory() {
        assertEquals(emptyMap<String, Focus>(), Backstack.decodeFocus(null))
        assertEquals(emptyMap<String, Focus>(), Backstack.decodeFocus(""))
        assertEquals(emptyMap<String, Focus>(), Backstack.decodeFocus("[1, 2, 3]"))
    }

    @Test
    fun rememberedTabsForSomethingGoneAreDropped() {
        val focus = mapOf(
            "h/alive" to Focus("t1", chosen = true),
            "h/merged-away" to Focus("t2", chosen = true),
        )
        val pruned = Backstack.prune(focus) { key, _ -> key == "h/alive" }
        assertEquals(setOf("h/alive"), pruned.keys)
    }

    // ---- Resolving a workspace's pane ----

    @Test
    fun theFocusWinsWhileTheAgentItNamesIsStillThere() {
        val terminals = listOf(
            terminal("blocked", activity = "blocked"),
            terminal("reading-this"),
        )
        assertEquals(
            "reading-this",
            Backstack.chooseFocus(terminals, Focus("reading-this", chosen = true)),
        )
    }

    /**
     * The case `docs/jobs-to-be-done.md` F4 is about, and the case it is not.
     *
     * A remembered diff you were reading comes back even though an agent in the
     * same workspace has since started asking for you — that is what resumable
     * means. A remembered pane that DIED overnight falls through to the rule
     * rather than to a blank screen, which is the other half of the same
     * sentence.
     */
    @Test
    fun aRememberedPaneThatDiedFallsThroughToTheRule() {
        val terminals = listOf(
            terminal("still-here"),
            terminal("needs-you", activity = "blocked"),
        )
        assertEquals(
            "needs-you",
            Backstack.chooseFocus(terminals, Focus("exited-overnight", chosen = true)),
        )
    }

    @Test
    fun withNoMemoryTheRuleDecides() {
        val terminals = listOf(
            terminal("first"),
            terminal("running", state = "running"),
            terminal("needs-you", activity = "blocked"),
        )
        assertEquals("needs-you", Backstack.chooseFocus(terminals, null))
        assertEquals("running", Backstack.chooseFocus(terminals.dropLast(1), null))
        assertEquals("first", Backstack.chooseFocus(listOf(terminal("first")), null))
    }

    @Test
    fun aWorkspaceWithNoPanesResolvesToNothing() {
        assertNull(Backstack.chooseFocus(emptyList(), Focus("t1", chosen = true)))
        assertNull(Backstack.chooseFocus(emptyList(), null))
    }

    // ---- Degrading a stack ----

    @Test
    fun aStackThatStillResolvesIsLeftAlone() {
        val stack = listOf(Route.Terminal("h", "w"), Route.Settings)
        assertEquals(stack, Backstack.truncate(stack) { true })
    }

    /**
     * Everything from the first dead route onwards goes.
     *
     * Truncating rather than filtering, because a stack is a story: if the
     * workspace you were reading was merged away, the runner settings screen
     * you had pushed on top of it is not where you meant to end up either.
     */
    @Test
    fun aDeadRouteTakesEverythingAboveItWithIt() {
        val stack = listOf(
            Route.Fleet,
            Route.Terminal("h", "merged-away"),
            Route.Settings,
            Route.RunnerSettings("h"),
        )
        val kept = Backstack.truncate(stack) { it != Route.Terminal("h", "merged-away") }
        assertEquals(listOf(Route.Fleet), kept)
    }

    /**
     * A route naming something gone degrades to the ROOT, never to a broken
     * screen and never to nothing.
     */
    @Test
    fun aStackWithNothingLeftDegradesToTheRoot() {
        val stack = listOf(Route.Terminal("removed-runner", "w"), Route.Settings)
        assertEquals(listOf(Backstack.ROOT), Backstack.truncate(stack) { false })
    }

    @Test
    fun truncationIsNeverEmpty() {
        for (stack in listOf(
            emptyList(),
            listOf(Route.Fleet),
            listOf(Route.Terminal("h", "w"), Route.Devices),
        )) {
            assertTrue(Backstack.truncate(stack) { false }.isNotEmpty())
            assertTrue(Backstack.truncate(stack) { true }.isNotEmpty())
        }
    }

    /**
     * The runner is part of what a route has to still name.
     *
     * A workspace id that survives on ANOTHER runner does not save this route,
     * which is the failure mode a fleet-wide app has and a one-runner-at-a-time
     * app does not.
     */
    @Test
    fun aRouteOnARemovedRunnerDoesNotResolveOffAnother() {
        val live = setOf(Route.Terminal("host-b", "shared-id"))
        val stack = listOf(Route.Fleet, Route.Terminal("host-a", "shared-id"))
        assertEquals(listOf(Route.Fleet), Backstack.truncate(stack) { it !is Route.Terminal || it in live })
    }

    // ---- The layering RootScreen draws from ----

    @Test
    fun onlyPushedScreensAreDrawnOverTheWorkspace() {
        for (route in listOf(Route.Onboarding, Route.Fleet, Route.Terminal("h", "w"))) {
            assertTrue("$route should be ground", !route.isOverlay)
        }
        for (route in listOf(
            Route.Settings,
            Route.RunnerSettings("h"),
            Route.Authorize,
            Route.Join,
            Route.AddDevice,
            Route.Devices,
        )) {
            assertTrue("$route should be an overlay", route.isOverlay)
        }
    }
}
