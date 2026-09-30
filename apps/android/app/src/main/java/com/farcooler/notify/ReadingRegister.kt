package com.farcooler.notify

/**
 * The terminal being read right now, and the only way to write it.
 *
 * Two screens decide what is being read: a worktree's pane at rest
 * (`WorktreeScreen`) and a workspace's orchestrator (`WorkspaceScreen`). Each
 * used to assign the register directly and clear it on its way out, so one
 * leaving wiped a claim the other had made since. A release now names what it
 * gives back, and gives back only that. The iPhone's `Notifier.claim` and
 * `release`.
 *
 * One register, held by `AppModel` and read by [Notifier] to suppress a banner
 * about the pane on screen. It is mirrored to the runner through
 * a [VisibleTerminalSink] (the connection), which [claim] and [release] with a
 * sink keep in step with this.
 */
class ReadingRegister {
    /** The pane being read, or null for none: the Changes tab, or nothing up. */
    @Volatile
    var current: String? = null
        private set

    /** [id] is the pane being read now, or null for none. */
    fun claim(id: String?) {
        current = id
    }

    /**
     * [id] isn't being read any more. A no-op when another pane has claimed
     * since, which is the whole difference from `claim(null)`. True when it
     * gave the claim back.
     */
    fun release(id: String): Boolean {
        if (current != id) return false
        current = null
        return true
    }
}

/**
 * What the runner is told is being read: `Connection`'s claim. An interface so
 * the two-sided rule below is tested without a live connection.
 */
interface VisibleTerminalSink {
    var visibleTerminal: String?

    /**
     * [id] isn't being read any more: gives the claim back if it is still
     * [id]'s, and does nothing if a pane claimed since (ov-69).
     */
    fun releaseVisible(id: String) {
        if (visibleTerminal == id) visibleTerminal = null
    }
}

/** [id] is the pane being read now on [sink], or null for none (the Changes tab). */
fun ReadingRegister.claim(sink: VisibleTerminalSink, id: String?) {
    claim(id)
    sink.visibleTerminal = id
}

/**
 * [id] isn't being read any more. Each side gives back only what is still
 * [id]'s, so a screen leaving after another has claimed (the orchestrator's
 * tab over a worktree, or the reverse) wipes nothing, on this phone or at the
 * runner.
 */
fun ReadingRegister.release(sink: VisibleTerminalSink, id: String) {
    release(id)
    sink.releaseVisible(id)
}
