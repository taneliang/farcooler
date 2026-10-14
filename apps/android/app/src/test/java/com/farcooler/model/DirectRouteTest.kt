package com.farcooler.model

import com.farcooler.data.Reach
import com.farcooler.data.Runner
import com.farcooler.net.Connection
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The owner's route rule on Android (ov-485): the tunnel first, the address
 * only when a person asks, and the question only where it can help. The same
 * cases as AgentKit's `TunnelRouteTests`.
 */
class DirectRouteTest {
    private val tunneled = Runner(id = "r", label = "Attic", reach = Reach.Tailcat("t"), user = "me")
    private val address = Reach.Direct("192.168.1.180", 22)

    @Test
    fun aFailedTunnelWithAnAddressAsksInTheOwnersWords() {
        val offer = DirectRoute.offer(tunneled, address, Connection.Failure.TUNNEL_NO_ANSWER)!!
        assertEquals(
            "Connecting through Tailcat failed. Connect directly to 192.168.1.180 instead?",
            offer.sentence,
        )
    }

    @Test
    fun nothingIsAskedWhereTheAddressCannotHelp() {
        val direct = tunneled.copy(reach = address)
        assertNull(DirectRoute.offer(direct, Reach.Tailcat("t"), Connection.Failure.UNREACHABLE))
        assertNull(DirectRoute.offer(tunneled, null, Connection.Failure.TUNNEL_NO_ANSWER))
        assertNull(DirectRoute.offer(tunneled, address, Connection.Failure.KEY_REJECTED))
        assertNull(DirectRoute.offer(tunneled, address, null))
    }

    @Test
    fun onlyAFailureBeforeTheRunnerOffersTheAddress() {
        val before = setOf(
            Connection.Failure.UNREACHABLE, Connection.Failure.NO_NODE_KEY, Connection.Failure.STOPPED,
            Connection.Failure.TUNNEL_NO_ANSWER, Connection.Failure.TUNNEL_RENDEZVOUS,
            Connection.Failure.TUNNEL_NOT_IN_THIS_BUILD, Connection.Failure.TUNNEL_UNSPECIFIED,
        )
        for (failure in Connection.Failure.entries) {
            assertEquals(failure.name, failure in before, DirectRoute.failedBeforeTheRunner(failure))
        }
    }

    @Test
    fun aRunnerThatAnsweredWithATokenIsGrantedTheTunnelFirst() {
        assertEquals(Reach.Tailcat("t") to address, DirectRoute.grant(address, null, "t", true))
    }

    @Test
    fun noTokenKeepsTheAddressAloneAndAHeldTokenIsNeverGranted() {
        assertEquals(address to null, DirectRoute.grant(address, Reach.Tailcat("held"), "", true))
    }

    @Test
    fun aTunneledRunnerStaysOnTheTunnelWithItsAddress() {
        assertEquals(Reach.Tailcat("its") to address, DirectRoute.grant(Reach.Tailcat("its"), address, "fresh", true))
    }

    @Test
    fun aDeviceWithNoNodeKeyGetsTheAddress() {
        assertEquals(address to null, DirectRoute.grant(Reach.Tailcat("its"), address, "", false))
        assertEquals(Reach.Tailcat("its") to null, DirectRoute.grant(Reach.Tailcat("its"), null, "", false))
        assertEquals(address to null, DirectRoute.grant(address, null, "t", false))
    }

    @Test
    fun theRouteIsNamedWithTheMoveToTheOther() {
        assertEquals("Through Tailcat", DirectRoute.label(Reach.Tailcat("t")))
        assertEquals("Directly to mini.local:2222", DirectRoute.label(Reach.Direct("mini.local", 2222)))
        assertEquals("Connect directly", DirectRoute.switchLabel(Reach.Tailcat("t")))
        assertEquals("Use Tailcat", DirectRoute.switchLabel(address))
        assertTrue(DirectRoute.failedBeforeTheRunner(Connection.Failure.NO_NODE_KEY))
        assertFalse(DirectRoute.failedBeforeTheRunner(Connection.Failure.OTHER))
    }
}
