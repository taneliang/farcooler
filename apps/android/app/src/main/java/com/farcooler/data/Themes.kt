package com.farcooler.data

import android.content.Context
import android.content.SharedPreferences
import com.farcooler.core.NativeClient
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

/**
 * A colour scheme: the terminal's palette, and which way the app around it
 * goes.
 *
 * The same shape the Mac and the iPhone decode, from the same source — the
 * built-in table in `farcooler_core::theme`. No client defines a colour of its
 * own, which is what stops "Nord" meaning three different things on three
 * screens.
 *
 * Colours are signed `Int` because Kotlin's `Int` is what Compose and the JNI
 * bridge both take; the bit pattern is the packed `0xRRGGBB` the core sends.
 */
@Serializable
data class Theme(
    val name: String,
    val dark: Boolean,
    val background: Int,
    val foreground: Int,
    val cursor: Int,
    /** Sixteen: ANSI 0-7 then 8-15. */
    val ansi: List<Int>,
) {
    /** The nineteen values `nativeSetPalette` expects, in its order. */
    fun packed(): IntArray = (ansi + listOf(foreground, background, cursor)).toIntArray()
}

@Serializable
private data class ThemeList(val themes: List<Theme> = emptyList())

/**
 * Every theme this device offers, and which one is in force.
 *
 * Two sources, one list. The built-ins come from the client core with no
 * connection at all — a phone on a train still has to render something — and
 * whatever a connected runner defines is merged on top, the runner winning a
 * name collision because it is the more specific statement.
 *
 * What is STORED is the name, never the colours: a host theme can be edited,
 * and a client that had cached its values would go on showing the old ones
 * forever.
 */
object Themes {
    private const val KEY = "app.theme"
    private const val FALLBACK_NAME = "Nord"

    /**
     * The one theme that exists before the core has been asked. Only covers
     * the microseconds before [initialize] runs, but a null here would be a
     * black screen.
     */
    private val fallback = Theme(
        name = FALLBACK_NAME,
        dark = true,
        background = 0x2E3440,
        foreground = 0xD8DEE9,
        cursor = 0xD8DEE9,
        ansi = listOf(
            0x3B4252, 0xBF616A, 0xA3BE8C, 0xEBCB8B, 0x81A1C1, 0xB48EAD, 0x88C0D0, 0xE5E9F0,
            0x4C566A, 0xBF616A, 0xA3BE8C, 0xEBCB8B, 0x81A1C1, 0xB48EAD, 0x8FBCBB, 0xECEFF4,
        ),
    )

    private val json = Json { ignoreUnknownKeys = true }
    private var prefs: SharedPreferences? = null
    private var builtIn: List<Theme> = listOf(fallback)

    private val _available = MutableStateFlow(listOf(fallback))
    val available: StateFlow<List<Theme>> = _available.asStateFlow()

    /**
     * Bumped whenever the colours in force change, so a live terminal
     * repaints. The same shape the font size setting already uses.
     */
    private val _revision = MutableStateFlow(0)
    val revision: StateFlow<Int> = _revision.asStateFlow()

    private val _selected = MutableStateFlow(FALLBACK_NAME)
    val selected: StateFlow<String> = _selected.asStateFlow()

    fun initialize(context: Context) {
        if (prefs != null) return
        prefs = context.applicationContext.getSharedPreferences("farcooler", Context.MODE_PRIVATE)
        builtIn = readBuiltIn()
        _available.value = builtIn
        _selected.value = prefs?.getString(KEY, FALLBACK_NAME) ?: FALLBACK_NAME
        _revision.value += 1
    }

    /**
     * The theme in force.
     *
     * Falls back rather than to nothing when a stored name no longer resolves:
     * a theme that vanished because a host's config file moved should cost you
     * your colours, not your terminal.
     */
    val current: Theme
        get() = _available.value.firstOrNull { it.name == _selected.value } ?: fallback

    fun select(name: String) {
        if (_selected.value == name) return
        prefs?.edit()?.putString(KEY, name)?.apply()
        _selected.value = name
        _revision.value += 1
    }

    /**
     * Merge in whatever the connected runner defines.
     *
     * Additive: the built-ins belong to this device and do not depend on which
     * runner it happens to be talking to, so switching runners must not empty
     * the picker.
     */
    fun merge(hostThemes: List<Theme>) {
        val merged = builtIn.toMutableList()
        for (theme in hostThemes) {
            // The host wins a name collision — it is the one somebody edited a
            // file on purpose to make.
            val index = merged.indexOfFirst { it.name == theme.name }
            if (index >= 0) merged[index] = theme else merged.add(theme)
        }
        if (merged == _available.value) return
        _available.value = merged
        _revision.value += 1
    }

    /** Opaque ARGB, for Compose, from the core's packed RGB. */
    fun opaque(rgb: Int): Int = rgb or 0xFF000000.toInt()

    private fun readBuiltIn(): List<Theme> {
        val raw = runCatching { NativeClient.nativeBuiltinThemes() }.getOrNull()
            ?: return listOf(fallback)
        val parsed = runCatching { json.decodeFromString(ThemeList.serializer(), raw) }.getOrNull()
        return parsed?.themes?.takeIf { it.isNotEmpty() } ?: listOf(fallback)
    }
}
