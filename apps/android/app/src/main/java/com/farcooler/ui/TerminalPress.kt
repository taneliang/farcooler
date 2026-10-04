package com.farcooler.ui

import com.farcooler.model.LinkOpen
import com.farcooler.model.TaskKeyLinks
import com.farcooler.net.TerminalSession

/**
 * A long press on terminal output (ov-215), out of [TerminalPane] and
 * [HeldLinkDialog] so a JVM test drives the same calls the pane makes: the
 * pane has no other seam, and Robolectric can't run this app's SDK on the
 * Java the build uses.
 */
object TerminalPress {
    /**
     * The link a long press on a cell lands on, asked with [linker]'s task
     * keys, or null, which pastes. The pane hands its own linker, so the keys
     * asked about are the boards this runner's connection has read.
     */
    fun linkAt(session: TerminalSession, linker: TaskKeyLinker, column: Int, row: Int): String? =
        session.linkAt(row, column, linker.index)

    /**
     * The dialog's Open. A task link opens its task in the app, through
     * [linker], and never through [openUri]: the system would hand
     * `farcooler://` to whichever channel's app claimed it. Anything else goes
     * to [openUri] through [LinkOpen]. Answers why it didn't open, or null.
     */
    fun open(link: String, linker: TaskKeyLinker, openUri: (String) -> Unit): String? {
        if (TaskKeyLinks.parse(link) != null) {
            linker.follow(link)
            return null
        }
        return LinkOpen.open(link, openUri)
    }
}
