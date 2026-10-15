package com.farcooler.account

/** One of a phone's three key slots during "Make a new key" (ov-569). */
enum class KeySlot { CURRENT, NEXT, PREVIOUS }

/** Where the slots live: [com.farcooler.data.Identity]'s Keystore-wrapped store. */
interface KeySlotStore {
    fun read(slot: KeySlot): String?
    /** False when the store refused, and then the slot is as it was. */
    fun write(value: String, slot: KeySlot): Boolean
    fun delete(slot: KeySlot)
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
        val next = store.read(KeySlot.NEXT) ?: return false
        val current = store.read(KeySlot.CURRENT) ?: return false
        if (current == next) {
            // Stopped after the swap: the old key is in `previous` already.
            if (store.read(KeySlot.PREVIOUS) == null) return false
            store.delete(KeySlot.NEXT)
            return true
        }
        when (store.read(KeySlot.PREVIOUS)) {
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
