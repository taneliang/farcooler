package com.farcooler.model

import kotlinx.serialization.Serializable

/**
 * Whose Claude login a pane's agent runs on (ov-597), as the runner names it.
 *
 * Null on the pane on a runner where only the owner has a login, and from a
 * runner before the field, so a single-user setup draws nothing. The Mac and
 * the iPhone say the same words (`PaneClaudeLogin.swift`).
 */
@Serializable
data class PaneClaudeLogin(
    /** The member's handle; empty for the runner's owner. */
    val handle: String = "",
    /** The name people read; empty for the owner, whose name the runner doesn't hold. */
    val name: String = "",
    /** Whether it is the runner's own login, the owner's. */
    val owner: Boolean = false,
) {
    /**
     * What the pane's info says: "Uses your Claude login" for the viewer's
     * own, else whose it is.
     *
     * [viewer] is who this phone is on the runner, as `member.list` says it:
     * empty for the owner, a member's handle, null if not known yet. Not known
     * names the person, which is true for everyone; it never says "your" on a
     * guess, and `(released)` is nobody's.
     */
    fun sentence(viewer: String?): String {
        if (viewer != null && viewer != RELEASED && viewer == (if (owner) "" else handle)) {
            return "Uses your Claude login"
        }
        if (owner) return "Uses the owner’s Claude login"
        val who = name.trim().ifEmpty { handle }
        return "Uses $who’s Claude login"
    }

    private companion object {
        const val RELEASED = "(released)"
    }
}
