package com.farcooler.data

import android.content.Context
import android.content.SharedPreferences
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
     * Whether to connect every configured machine at once.
     *
     * The Mac does this unconditionally — see
     * `docs/superpowers/specs/2026-08-03-every-machine-in-one-fleet-design.md`
     * — and the argument for it is stronger on a phone, where switching costs a
     * sheet and two taps. It is a setting rather than a rule only because a
     * phone pays for each extra SSH session in radio wake-ups: someone with six
     * machines on a train may want one.
     */
    private val _allMachinesAtOnce = MutableStateFlow(preferences.getBoolean(KEY_ALL_MACHINES, true))
    val allMachinesAtOnce: StateFlow<Boolean> = _allMachinesAtOnce.asStateFlow()

    /**
     * Whether opening a terminal reshapes the pane to this screen.
     *
     * A tmux pane is shared: resizing it reflows it for every other client
     * attached to that window, the Mac included. iOS made this automatic after
     * judging unreadably tiny text the worse failure — but a phone is much
     * narrower than a tablet, and someone using Far Cooler beside a Mac on the
     * same worktree may well prefer to read a squeezed screen over squeezing
     * everyone else's. So it is a choice, defaulting to the behaviour iOS
     * settled on.
     */
    private val _reshapePanes = MutableStateFlow(preferences.getBoolean(KEY_RESHAPE, true))
    val reshapePanes: StateFlow<Boolean> = _reshapePanes.asStateFlow()

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

    fun setAllMachinesAtOnce(on: Boolean) {
        _allMachinesAtOnce.value = on
        preferences.edit().putBoolean(KEY_ALL_MACHINES, on).apply()
    }

    fun setReshapePanes(on: Boolean) {
        _reshapePanes.value = on
        preferences.edit().putBoolean(KEY_RESHAPE, on).apply()
    }

    companion object {
        /**
         * Matches the size the Apple apps render at, so the same terminal on
         * the same machine is the same size wherever you look at it.
         */
        const val DEFAULT_FONT_SIZE = 13f
        const val MIN_FONT_SIZE = 9f
        const val MAX_FONT_SIZE = 22f

        private const val KEY_FONT = "terminalFont"
        private const val KEY_FONT_SIZE = "terminalFontSize"
        private const val KEY_ATTENTION = "notifyOnAttention"
        private const val KEY_DONE = "notifyOnDone"
        private const val KEY_ALL_MACHINES = "allMachinesAtOnce"
        private const val KEY_RESHAPE = "reshapePanes"
    }
}
