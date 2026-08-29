package com.farcooler.net

import kotlinx.serialization.Serializable

/**
 * One captured screen, as the host describes it.
 *
 * Internal rather than private to [TerminalSession] so the decode can be
 * pinned by a test. It earned that: the first Android build declared
 * [revision] as a `Long` and every second keystroke blanked the terminal with
 * a parse error, because a `Long` cannot hold half the values the host sends.
 */
@Serializable
data class ScreenResponse(
    val contents: String = "",
    val columns: Int = 80,
    val rows: Int = 24,
    val cursorColumn: Int = 0,
    val cursorRow: Int = 0,
    /**
     * A cheap identity for this screen, handed back on the next ask so the host
     * can answer "unchanged" in a hundred bytes instead of resending a capture
     * nothing did anything with.
     *
     * **Unsigned, and it has to be.** This is an FNV-1a hash over the capture
     * and the cursor (`crates/daemon/src/rpc.rs`, `screen_revision`), so it
     * uses the whole `u64` range — about half of all screens hash above
     * `Long.MAX_VALUE`. Declared as a `Long` it does not merely truncate: the
     * JSON parser refuses the literal outright, the whole response fails, and
     * the terminal shows "Could not load" instead of a screen. Every keystroke
     * changes the screen, so every keystroke re-rolled that coin.
     *
     * The two other `u64`s this client reads — an agent stream's `epoch` and an
     * event's `seq` — stay `Long` on purpose. Those are counters that start at
     * zero and step by one, so the range is theoretical; this one is a hash,
     * where the top half is not an edge case but half the values.
     */
    val revision: ULong = 0uL,
    val unchanged: Boolean = false,
    /** The escape sequences that put a fresh emulator into the program's modes. */
    val modes: String? = null,
    /**
     * The pane's scrollback, base64, and ready to feed.
     *
     * Present only when it was asked for — see `TerminalSession.HISTORY_LINES`
     * — and empty for a pane that has none, or one on the alternate screen,
     * which has no history of its own. The host has already repaired the bare
     * line feeds, appended the colour reset the last history line left set, and
     * decided the alternate-screen case, so what arrives here needs no repair:
     * it is the same assembly the daemon writes down a stream (`replay` in
     * `crates/daemon/src/runtime.rs`).
     *
     * Nullable because a runner too old to know the field omits it entirely,
     * which is exactly the behaviour that keeps this app working against a
     * daemon that has not been updated.
     */
    val history: String? = null,
)
