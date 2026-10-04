package com.farcooler.ui

import android.content.SharedPreferences
import com.farcooler.model.BoardReads
import com.farcooler.model.ReadsRaise
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The phone's real read store (ov-113): what `BoardReadsKeeperTest` proves
 * against the in-memory one, proven against the preferences encoding: the
 * made-up flag, the opened marks' `<id>=<ms>` strings, pending, and the
 * upload and floor-set flags.
 */
class PrefsBoardReadsTest {
    /** Preferences as a JVM can hold them: a map, with `apply` writing at once. */
    private class FakePrefs : SharedPreferences {
        val map = mutableMapOf<String, Any?>()
        override fun getAll(): MutableMap<String, *> = map
        override fun getString(key: String, defValue: String?) = map[key] as? String ?: defValue
        @Suppress("UNCHECKED_CAST")
        override fun getStringSet(key: String, defValues: MutableSet<String>?) = map[key] as? MutableSet<String> ?: defValues
        override fun getInt(key: String, defValue: Int) = map[key] as? Int ?: defValue
        override fun getLong(key: String, defValue: Long) = map[key] as? Long ?: defValue
        override fun getFloat(key: String, defValue: Float) = map[key] as? Float ?: defValue
        override fun getBoolean(key: String, defValue: Boolean) = map[key] as? Boolean ?: defValue
        override fun contains(key: String) = map.containsKey(key)
        override fun registerOnSharedPreferenceChangeListener(l: SharedPreferences.OnSharedPreferenceChangeListener?) {}
        override fun unregisterOnSharedPreferenceChangeListener(l: SharedPreferences.OnSharedPreferenceChangeListener?) {}
        override fun edit(): SharedPreferences.Editor = object : SharedPreferences.Editor {
            val staged = mutableMapOf<String, Any?>()
            val removed = mutableSetOf<String>()
            private fun stage(key: String, v: Any?) = apply { staged[key] = v; removed -= key }
            override fun putString(key: String, value: String?) = stage(key, value)
            override fun putStringSet(key: String, values: MutableSet<String>?) = stage(key, values?.toMutableSet())
            override fun putInt(key: String, value: Int) = stage(key, value)
            override fun putLong(key: String, value: Long) = stage(key, value)
            override fun putFloat(key: String, value: Float) = stage(key, value)
            override fun putBoolean(key: String, value: Boolean) = stage(key, value)
            override fun remove(key: String) = apply { removed += key; staged -= key }
            override fun clear() = apply { map.clear() }
            override fun commit(): Boolean { apply(); return true }
            override fun apply() {
                removed.forEach { map.remove(it) }
                map.putAll(staged)
            }
        }
    }

    private val host = "h"
    private val ws = "w"
    private val now = 1_800_000_000_000L

    @Test
    fun aFirstLookIsMadeUpSoNothingIsKept() {
        val store = PrefsBoardReads(FakePrefs())
        assertEquals(now - 86_400_000, store.load(host, ws, now).floorMs)
        assertNull("a first look nobody saw as Unread is no kept state", store.keptReads(host, ws))
    }

    @Test
    fun marksAreKeptAndASavedFloorIsRememberedAsMovedNotMadeUp() {
        val prefs = FakePrefs()
        val store = PrefsBoardReads(prefs)
        store.load(host, ws, now)
        store.save(BoardReads(now - 1000, mapOf("a" to now + 5, "gone" to now - 2000)), host, ws)
        val kept = store.keptReads(host, ws)!!
        assertEquals(now - 1000, kept.floorMs)
        assertEquals("a mark under the floor is dropped", mapOf("a" to now + 5), kept.opened)
        assertEquals(setOf("a=${now + 5}"), prefs.getStringSet("board.read.h.w.opened", null))
        assertEquals(kept, PrefsBoardReads(prefs).load(host, ws, now + 1))
    }

    @Test
    fun marksWithAMadeUpFloorAreKeptWithoutIt() {
        val store = PrefsBoardReads(FakePrefs())
        store.load(host, ws, now)
        store.save(BoardReads(store.load(host, ws, now).floorMs, mapOf("a" to now)), host, ws)
        val kept = store.keptReads(host, ws)!!
        assertEquals(Long.MIN_VALUE, kept.floorMs)
        assertEquals(mapOf("a" to now), kept.opened)
    }

    @Test
    fun pendingMarksSurviveARelaunchAndAnEmptyOneClears() {
        val prefs = FakePrefs()
        val pending = ReadsRaise(floorMs = 9, opened = mapOf("a" to 5L, "b=odd" to 7L))
        PrefsBoardReads(prefs).savePending(pending, host, ws)
        assertEquals(pending, PrefsBoardReads(prefs).loadPending(host, ws))
        PrefsBoardReads(prefs).savePending(ReadsRaise(), host, ws)
        assertTrue(PrefsBoardReads(prefs).loadPending(host, ws).isEmpty)
        assertNull("no floor is none, not zero", PrefsBoardReads(prefs).loadPending(host, ws).floorMs)
    }

    @Test
    fun theUploadAndFloorSetFlagsAreKeptPerBoard() {
        val prefs = FakePrefs()
        val store = PrefsBoardReads(prefs)
        assertFalse(store.isUploaded(host, ws))
        assertFalse("a floor an older build left was not set on purpose", store.floorWasSet(host, ws))
        store.markUploaded(host, ws)
        store.markFloorSet(host, ws)
        assertTrue(PrefsBoardReads(prefs).isUploaded(host, ws))
        assertTrue(PrefsBoardReads(prefs).floorWasSet(host, ws))
        assertFalse(PrefsBoardReads(prefs).isUploaded(host, "other"))
    }
}
