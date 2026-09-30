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
 * `Connection.visibleTerminal`, which `AppModel.claimReading` and
 * `releaseReading` keep in step with this.
 */
class ReadingRegister {
    /** The pane being read, or null for none: the Changes tab, or nothing up. */
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
