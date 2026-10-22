package com.farcooler.data

/**
 * Whether the app around the terminal is light or dark (ov-690).
 *
 * The Mac's `Appearance` (Preferences.swift), by name, options and label: Theme,
 * System, Light, Dark. One difference, on purpose: the Mac defaults to Dark and
 * this defaults to [SYSTEM], because the conversation is a chat and a chat
 * follows the phone. The terminal is never affected: it draws the chosen theme's
 * palette whatever this says.
 */
enum class Appearance(val wire: String, val label: String) {
    /** Whichever way the chosen terminal theme goes: the app's behavior before this setting existed. */
    THEME("theme", "Theme"),

    /** The phone's own light or dark mode, live. */
    SYSTEM("system", "System"),
    LIGHT("light", "Light"),
    DARK("dark", "Dark");

    /**
     * Which way the app goes. Both inputs are passed in rather than read, so
     * the answer is a function a JVM test can check: [systemDark] is the
     * phone's mode right now, [themeDark] is `Themes.current.dark`.
     */
    fun dark(systemDark: Boolean, themeDark: Boolean): Boolean = when (this) {
        THEME -> themeDark
        SYSTEM -> systemDark
        LIGHT -> false
        DARK -> true
    }

    companion object {
        val DEFAULT = SYSTEM

        /** An unknown or missing stored value reads as the default, never as a crash. */
        fun parse(raw: String?): Appearance = entries.firstOrNull { it.wire == raw } ?: DEFAULT
    }
}
