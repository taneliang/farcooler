package com.farcooler.model

import com.farcooler.ui.Backstack
import com.farcooler.ui.Route
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Add Teammate from People (ov-865): who is offered it, and where it goes. */
class AddTeammateRouteTest {
    private val bob = RunnerTeam.Member("bob", "Bob", RunnerTeam.Role.MEMBER)

    @Test
    fun `the owner's phone is offered Add Teammate alone on a runner and on a shared one`() {
        assertTrue(RunnerTeam(emptyList(), you = "").offersAddTeammate)
        assertTrue(RunnerTeam(listOf(bob), you = "").offersAddTeammate)
    }

    @Test
    fun `a member's phone is not offered it`() {
        assertFalse(RunnerTeam(listOf(bob), you = "bob").offersAddTeammate)
    }

    @Test
    fun `from a runner's settings it opens the add-a-device screen and back returns`() {
        val settings = listOf(Route.NeedsYou, Route.Settings, Route.RunnerSettings("host-a"))
        val opened = settings + Route.AddDevice
        assertEquals(Route.AddDevice, Backstack.decodeStack(Backstack.encodeStack(opened))!!.last())
    }
}
