package com.farcooler.account

/** One of a phone's three key slots during "Make a new key" (ov-569). */
enum class KeySlot { CURRENT, NEXT, PREVIOUS }

/** Where the slots live: [com.farcooler.data.Identity]'s Keystore-wrapped store. */
interface KeySlotStore {
    fun read(slot: KeySlot): SlotRead
    /** False when the store refused, and then the slot is as it was. */
    fun write(value: String, slot: KeySlot): Boolean
    fun delete(slot: KeySlot)
}

/** What reading a slot found. [Failed] is not [Empty]: the slot may hold a key that could not be opened just now. */
sealed interface SlotRead {
    data object Empty : SlotRead
    data class Held(val value: String) : SlotRead
    data object Failed : SlotRead
}

/**
 * Putting the new key in use, so that stopping at any step (a crash, a refused
 * write) and running it again ends the same way, and neither key is ever lost
 * (fix round 1, M1). The same order as AgentKit's `KeySlotPromotion.swift`:
 * the old key goes to `previous`, never over an old key already there; then
 * `current` becomes the new key; then the waiting copy is deleted. Run again
 * after `current` already holds the new key, it only deletes the waiting copy.
 */
object KeySlotPromotion {
    fun promote(store: KeySlotStore): Boolean {
        // Any slot that cannot be read stops the whole thing: a read that failed
        // is not an empty slot, and treating it as one writes over a key.
        val next = (store.read(KeySlot.NEXT) as? SlotRead.Held)?.value ?: return false
        val current = (store.read(KeySlot.CURRENT) as? SlotRead.Held)?.value ?: return false
        val previous = when (val r = store.read(KeySlot.PREVIOUS)) {
            is SlotRead.Failed -> return false
            is SlotRead.Held -> r.value
            is SlotRead.Empty -> null
        }
        if (current == next) {
            // Stopped after the swap: the old key is in `previous` already.
            if (previous == null) return false
            store.delete(KeySlot.NEXT)
            return true
        }
        when (previous) {
            null -> if (!store.write(current, KeySlot.PREVIOUS)) return false
            current -> Unit
            // Another old key, still kept for some runner: never overwritten.
            else -> return false
        }
        if (!store.write(next, KeySlot.CURRENT)) return false
        store.delete(KeySlot.NEXT)
        return true
    }
}
