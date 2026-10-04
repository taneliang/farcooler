package com.farcooler.net

import com.farcooler.model.Capability
import com.farcooler.model.DaemonBuild
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * A runner's build is read again every time its link comes up, and a read from
 * the previous link cannot land on the new one. The iPhone's
 * `Connection.forgetDaemonBuild` is the same rule.
 */
class DaemonBuildSlotTest {
    private val old = DaemonBuild("1.0", true, "macos", capabilities = setOf("tasks"))
    private val upgraded = DaemonBuild("1.1", true, "macos", capabilities = setOf("tasks", "watching"))

    /**
     * The bug: a runner upgraded while the app ran reconnected, and its new
     * capabilities stayed shut until relaunch because the old build was kept.
     */
    @Test
    fun `a new link forgets the build the last one read`() {
        val slot = DaemonBuildSlot()
        slot.linkCameUp()
        assertTrue(slot.land(slot.link, old))

        slot.linkCameUp()
        assertNull(slot.current.value)
        assertTrue(slot.land(slot.link, upgraded))
        assertEquals(true, slot.current.value?.can(Capability.WATCHING))
    }

    @Test
    fun `a read that set out on the previous link is dropped`() {
        val slot = DaemonBuildSlot()
        slot.linkCameUp()
        val readOn = slot.link

        slot.linkCameUp()
        assertFalse(slot.land(readOn, old))
        assertNull(slot.current.value)
    }

    /** The runner's name survives the round trip after a reconnect; its gates do not. */
    @Test
    fun `the last build is kept through a reconnect`() {
        val slot = DaemonBuildSlot()
        slot.linkCameUp()
        slot.land(slot.link, old)

        slot.linkCameUp()
        assertNull(slot.current.value)
        assertEquals(old, slot.last.value)
    }
}
