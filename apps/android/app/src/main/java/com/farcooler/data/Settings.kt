package com.farcooler.data

import android.content.Context
import android.content.SharedPreferences
import java.net.URI
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * The terminal's typeface: bundled Iosevka, or the system's own monospace.
 *
 * Not a free-form font picker. The terminal draws a fixed grid of one glyph per
 * cell, and anything that is not genuinely monospaced would misalign the exact
 * thing a terminal is. Two options is the whole space worth offering: Iosevka
 * for the box-drawing and powerline glyphs coding agents print constantly, and
 * the system face as the one that needs no bundle to have shipped correctly.
 */
enum class TerminalFontChoice(val wire: String, val label: String) {
    IOSEVKA("iosevka", "Iosevka"),
    SYSTEM("system", "System Monospaced");

    companion object {
        fun parse(raw: String?): TerminalFontChoice =
            entries.firstOrNull { it.wire == raw } ?: IOSEVKA
    }
}

/**
 * Everything this app lets you set, and nothing else.
 *
 * A single object with flows rather than a preference screen reading and
 * writing keys by hand: the terminal redraws from these on every frame, so what
 * matters is that a change is observable, not that it is transactional.
 */
class Settings(context: Context) {
    private val preferences: SharedPreferences =
        context.applicationContext.getSharedPreferences("farcooler.settings", Context.MODE_PRIVATE)

    private val _font = MutableStateFlow(
        TerminalFontChoice.parse(preferences.getString(KEY_FONT, null))
    )
    val font: StateFlow<TerminalFontChoice> = _font.asStateFlow()

    private val _fontSize = MutableStateFlow(preferences.getFloat(KEY_FONT_SIZE, DEFAULT_FONT_SIZE))
    val fontSize: StateFlow<Float> = _fontSize.asStateFlow()

    /**
     * Both default on, because an agent that needs you and cannot say so is the
     * failure this feature exists to prevent. Someone who finds it noisy turns
     * it off having seen what it does.
     */
    private val _notifyOnAttention = MutableStateFlow(preferences.getBoolean(KEY_ATTENTION, true))
    val notifyOnAttention: StateFlow<Boolean> = _notifyOnAttention.asStateFlow()

    private val _notifyOnDone = MutableStateFlow(preferences.getBoolean(KEY_DONE, true))
    val notifyOnDone: StateFlow<Boolean> = _notifyOnDone.asStateFlow()

    /**
     * Whether to connect every configured runner at once.
     *
     * The Mac does this unconditionally — see
     * `docs/superpowers/specs/2026-08-03-every-machine-in-one-fleet-design.md`
     * — and the argument for it is stronger on a phone, where switching costs a
     * sheet and two taps. It is a setting rather than a rule only because a
     * phone pays for each extra SSH session in radio wake-ups: someone with six
     * runners on a train may want one.
     */
    private val _allRunnersAtOnce = MutableStateFlow(preferences.getBoolean(KEY_ALL_RUNNERS, true))
    val allRunnersAtOnce: StateFlow<Boolean> = _allRunnersAtOnce.asStateFlow()

    /**
     * Whether opening a terminal reshapes the pane to this screen.
     *
     * A tmux pane is shared: resizing it reflows it for every other client
     * attached to that window, the Mac included. iOS made this automatic after
     * judging unreadably tiny text the worse failure — but a phone is much
     * narrower than a tablet, and someone using Far Cooler beside a Mac on the
     * same workspace may well prefer to read a squeezed screen over squeezing
     * everyone else's. So it is a choice, defaulting to the behavior iOS
     * settled on.
     */
    private val _reshapePanes = MutableStateFlow(preferences.getBoolean(KEY_RESHAPE, true))
    val reshapePanes: StateFlow<Boolean> = _reshapePanes.asStateFlow()

    /**
     * Where tunneled runners and this device meet, or empty for the one the app
     * ships with.
     *
     * A tunneled runner is not reachable by address — a device and a runner find
     * each other through a rendezvous service, and EVERY tunneled connection
     * goes through it rather than only the ones that could not go direct. The
     * service this app ships with is documented as best-effort and revocable at
     * any time, so this exists for one day: the day it stops answering. Without
     * it, that day costs a Play release, an App Store review, a Mac release and
     * a visit to every runner in the fleet. With it, it costs a setting.
     *
     * **Empty is the answer, for almost everybody, forever.** Nothing in this
     * product says anyone should run their own, and the screen that shows this
     * says so rather than implying otherwise.
     *
     * Normalized on the way in AND on the way out — see [derpMapSetting] — so a
     * value that arrived some other way, from an older build or a restored
     * backup, still cannot become a rendezvous nobody chose. This is the same
     * setting the Apple apps keep in `Account.derpMap`, held to the same rules.
     */
    private val _derpMap =
        MutableStateFlow(derpMapSetting(preferences.getString(KEY_DERP_MAP, null)))
    val derpMap: StateFlow<String> = _derpMap.asStateFlow()

    fun setFont(choice: TerminalFontChoice) {
        _font.value = choice
        preferences.edit().putString(KEY_FONT, choice.wire).apply()
    }

    fun setFontSize(size: Float) {
        val clamped = size.coerceIn(MIN_FONT_SIZE, MAX_FONT_SIZE)
        _fontSize.value = clamped
        preferences.edit().putFloat(KEY_FONT_SIZE, clamped).apply()
    }

    fun setNotifyOnAttention(on: Boolean) {
        _notifyOnAttention.value = on
        preferences.edit().putBoolean(KEY_ATTENTION, on).apply()
    }

    fun setNotifyOnDone(on: Boolean) {
        _notifyOnDone.value = on
        preferences.edit().putBoolean(KEY_DONE, on).apply()
    }

    fun setAllRunnersAtOnce(on: Boolean) {
        _allRunnersAtOnce.value = on
        preferences.edit().putBoolean(KEY_ALL_RUNNERS, on).apply()
    }

    fun setReshapePanes(on: Boolean) {
        _reshapePanes.value = on
        preferences.edit().putBoolean(KEY_RESHAPE, on).apply()
    }

    /**
     * Take a rendezvous, or go back to the one the app ships with.
     *
     * What is stored is what [derpMapSetting] answered, not what was typed:
     * anything it refuses is stored as empty, and empty means the default. A
     * value that saved and then silently did nothing would be worse than one
     * that would not save, because a tunnel meeting nowhere times out rather
     * than refusing — there would be nothing on any screen to read.
     */
    fun setDerpMap(url: String) {
        val usable = derpMapSetting(url)
        _derpMap.value = usable
        preferences.edit().putString(KEY_DERP_MAP, usable).apply()
    }

    companion object {
        /**
         * Matches the size the Apple apps render at, so the same terminal on
         * the same runner is the same size wherever you look at it.
         */
        const val DEFAULT_FONT_SIZE = 13f
        const val MIN_FONT_SIZE = 9f
        const val MAX_FONT_SIZE = 22f

        private const val KEY_FONT = "terminalFont"
        private const val KEY_FONT_SIZE = "terminalFontSize"
        private const val KEY_ATTENTION = "notifyOnAttention"
        private const val KEY_DONE = "notifyOnDone"
        /** The stored spelling stays, so an existing install keeps its answer. */
        private const val KEY_ALL_RUNNERS = "allMachinesAtOnce"
        private const val KEY_RESHAPE = "reshapePanes"
        private const val KEY_DERP_MAP = "derpMap"

        /**
         * The DERP map worth using, out of whatever somebody typed.
         *
         * Anything that is not an `https` URL comes back empty, and empty means
         * the tunnel library's own default. Refused rather than repaired: a
         * rendezvous is where a device and a runner agree to meet, and a value
         * this could not read is a value nobody deliberately chose. A scheme
         * typed in capitals is refused for that reason too, rather than being
         * quietly rewritten into one nobody looked at.
         *
         * `https` and not merely on principle. A map fetched over cleartext is
         * a map anybody on the path can rewrite, and rewriting it moves both
         * ends of a tunnel onto a rendezvous of the attacker's choosing — the
         * one thing this whole setting must not make possible.
         *
         * Whitespace is refused rather than trimmed out of the middle. The
         * tunnel library refuses a URL carrying a space — one of its backends
         * sends it to a subprocess over a line protocol whose fields are
         * separated by spaces, so a second field would arrive as a command
         * nobody issued — and this side refusing first is what keeps such a
         * value from being saved, ignored, and never mentioned again.
         *
         * The same answer `Account.derpMapSetting` gives on the Apple apps.
         * Two implementations of one rule, because there is no shared code
         * between a Kotlin `SharedPreferences` and a Swift `UserDefaults`; the
         * rule itself is enforced once more underneath, in
         * `farcooler_tailcat::set_derp_map_url`, which is what actually holds
         * the line for every platform.
         */
        fun derpMapSetting(typed: String?): String {
            val trimmed = typed?.trim() ?: return ""
            if (trimmed.isEmpty() || trimmed.any { it.isWhitespace() }) return ""
            val url = runCatching { URI(trimmed) }.getOrNull() ?: return ""
            if (url.scheme != "https") return ""
            if (url.host.isNullOrEmpty()) return ""
            return trimmed
        }
    }
}
