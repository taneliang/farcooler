package com.farcooler.account

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Putting the new key in use survives stopping at every step (ov-569 fix round 1, M1). */
class KeySlotPromotionTest {
    /** Slots in memory that stop working after [budget] writes and deletes, as a crash would leave them. */
    private class Slots(val values: MutableMap<KeySlot, String>, var budget: Int = Int.MAX_VALUE) : KeySlotStore {
        /** Slots whose next read fails once, as a busy Keystore would. */
        val failingOnce = mutableSetOf<KeySlot>()
        override fun read(slot: KeySlot): SlotRead {
            if (failingOnce.remove(slot)) return SlotRead.Failed
            return values[slot]?.let { SlotRead.Held(it) } ?: SlotRead.Empty
        }
        override fun write(value: String, slot: KeySlot): Boolean {
            if (budget <= 0) return false
            budget--
            values[slot] = value
            return true
        }
        override fun delete(slot: KeySlot) {
            if (budget <= 0) return
            budget--
            values.remove(slot)
        }
    }

    @Test
    fun stoppedAfterAnyStepRunningItAgainPutsTheNewKeyInUseAndKeepsTheOldOne() {
        for (stop in 0..3) {
            val slots = Slots(mutableMapOf(KeySlot.CURRENT to "old", KeySlot.NEXT to "new"), budget = stop)
            KeySlotPromotion.promote(slots)
            assertTrue("stopped after $stop", slots.values.values.containsAll(listOf("old", "new")))
            slots.budget = Int.MAX_VALUE
            val finished = KeySlot.NEXT !in slots.values
            assertEquals("stopped after $stop", !finished, KeySlotPromotion.promote(slots))
            assertEquals("stopped after $stop", mapOf(KeySlot.CURRENT to "new", KeySlot.PREVIOUS to "old"), slots.values)
            assertFalse(KeySlotPromotion.promote(slots))
            assertEquals(mapOf(KeySlot.CURRENT to "new", KeySlot.PREVIOUS to "old"), slots.values)
        }
    }

    @Test
    fun anOldKeyAlreadyKeptForARunnerIsNeverWrittenOver() {
        val before = mapOf(KeySlot.CURRENT to "middle", KeySlot.NEXT to "new", KeySlot.PREVIOUS to "oldest")
        val slots = Slots(before.toMutableMap())
        assertFalse(KeySlotPromotion.promote(slots))
        assertEquals(before, slots.values)
    }

    @Test
    fun aSlotThatFailsToReadIsNotEmptyAndNothingIsWritten() {
        for (failing in KeySlot.values()) {
            val before = mapOf(KeySlot.CURRENT to "old", KeySlot.NEXT to "new", KeySlot.PREVIOUS to "kept")
            val slots = Slots(before.toMutableMap())
            slots.failingOnce += failing
            assertFalse("$failing", KeySlotPromotion.promote(slots))
            assertEquals("$failing", before, slots.values)
        }
    }

    @Test
    fun aPreviousSlotThatFailsToReadIsNeverOverwrittenWithCurrent() {
        val before = mapOf(KeySlot.CURRENT to "old", KeySlot.NEXT to "new", KeySlot.PREVIOUS to "kept")
        val slots = Slots(before.toMutableMap())
        slots.failingOnce += KeySlot.PREVIOUS
        KeySlotPromotion.promote(slots)
        assertEquals("kept", slots.values[KeySlot.PREVIOUS])
        assertTrue(KeySlotPromotion.promote(slots).not())
    }
}
