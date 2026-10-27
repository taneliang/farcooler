package com.farcooler.net

import com.farcooler.data.Reach
import com.farcooler.data.Runner
import java.io.File
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.test.StandardTestDispatcher
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A phone whose access is gone dials once per recheck (ov-821): the dials nobody asked for (the app coming forward,
 * the network returning) go through [Connection.reconnectIfDue], which holds off while the last dial is under five
 * minutes old. A person's tap, [Connection.reconnectNow], doesn't.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ReconnectHoldOffTest {
    private val runner = Runner(id = "r1", label = "studio", reach = Reach.Direct("192.168.1.10", 22), user = "me")

    /** A connection whose scope never runs, so a dial it decides on shows only as its phase. */
    private fun connection(): Connection {
        val scope = CoroutineScope(StandardTestDispatcher())
        return Connection(
            runner,
            object : com.farcooler.data.ReviewStorage {
                override fun read(key: String): String? = null
                override fun write(key: String, value: String) = Unit
                override fun remove(key: String) = Unit
            },
            MutableStateFlow(""),
            scope,
        )
    }

    private fun settled(connection: Connection, at: Long) {
        val refusals = connection.keyRefusals
        refusals.letIn()
        refusals.release()
        refusals.dialing(at)
        connection.nowMs = { at }
    }

    @Test
    fun aSettledPhoneHoldsOffADialNobodyAskedForAndDialsOnceTheRecheckIsDue() {
        val connection = connection()
        settled(connection, at = 1_000_000L)

        connection.nowMs = { 1_000_000L + 120_000 }
        connection.reconnectIfDue()
        assertTrue("dialed two minutes after the last dial", connection.phase.value is Connection.Phase.Connecting)

        connection.nowMs = { 1_000_000L + 301_000 }
        connection.reconnectIfDue()
        assertEquals(Connection.Phase.Reconnecting(0), connection.phase.value)
    }

    @Test
    fun aTapDialsAtOnceAndAPhoneNotSettledIsNeverHeldOff() {
        val tapped = connection()
        settled(tapped, at = 1_000_000L)
        tapped.reconnectNow()
        assertEquals(Connection.Phase.Reconnecting(0), tapped.phase.value)

        val fresh = connection()
        fresh.reconnectIfDue()
        assertEquals(Connection.Phase.Reconnecting(0), fresh.phase.value)
    }

    /**
     * The call sites, which nothing else reaches without a live core: the network returning ([FleetRepository]) and
     * the app coming forward ([Connection.dialOnReturn]) must ask [Connection.reconnectIfDue], never dial outright.
     */
    @Test
    fun theNetworkAndTheForegroundAskReconnectIfDue() {
        val fleet = File("src/main/java/com/farcooler/net/FleetRepository.kt").readText()
        val reconnectAll = fleet.substringAfter("fun reconnectAll()").substringBefore("}")
        assertTrue(reconnectAll, reconnectAll.contains("reconnectIfDue()"))
        assertFalse(reconnectAll, reconnectAll.contains("reconnectNow()"))

        val connection = File("src/main/java/com/farcooler/net/Connection.kt").readText()
        val onReturn = connection.substringAfter("private fun dialOnReturn(").substringBefore("\n    }\n")
        assertTrue(onReturn, onReturn.contains("reconnectIfDue()"))
        assertFalse(onReturn, onReturn.contains("reconnectNow()"))
    }
}
