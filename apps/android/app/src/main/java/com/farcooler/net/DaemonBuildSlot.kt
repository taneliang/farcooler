package com.farcooler.net

import com.farcooler.model.DaemonBuild
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * The build a runner reported, read again every time its link comes up.
 *
 * It used to be read once per [Connection] and kept, so the first answer stood
 * for the life of the process. A runner upgraded while this app was running
 * reconnected and went on being described by its old build, and every gate on
 * its capabilities (`watching`, `needs_you`, `tasks`, the admin scope) stayed as
 * the old build had them until the app was relaunched. The iPhone's
 * `Connection.forgetDaemonBuild` is the same rule.
 *
 * [linkCameUp] is called the moment a link comes up, by a start and a reconnect
 * alike, and not when it goes down: a runner that is reconnecting keeps showing
 * what it last was. Between the clear and the read landing, a capability gate
 * answers as for a runner nobody has asked, which is the refusing answer, for
 * one round trip.
 */
class DaemonBuildSlot {
    private val _current = MutableStateFlow<DaemonBuild?>(null)

    /** The build read on the link that is up now, or null until it lands. Every capability gate asks this. */
    val current: StateFlow<DaemonBuild?> = _current.asStateFlow()

    private val _last = MutableStateFlow<DaemonBuild?>(null)

    /**
     * The last build any link to this runner reported, kept through a
     * reconnect. For naming the runner (its `runnerId`) only, never for a
     * capability gate: [current]'s clear is what makes a reconnected runner
     * prove itself again.
     */
    val last: StateFlow<DaemonBuild?> = _last.asStateFlow()

    /**
     * Which link [current] is being read for. Bumped by [linkCameUp], so a read
     * that set out on the previous link and answers after the new one came up
     * is dropped rather than installing the old build over the clear.
     */
    var link: Int = 0
        private set

    /** A new link: forget what the previous one said, so the next read asks again. */
    fun linkCameUp() {
        _current.value = null
        link += 1
    }

    /**
     * Install [build], read by a call that set out on [readOn]. Returns false,
     * and installs nothing, when a newer link has come up since.
     */
    fun land(readOn: Int, build: DaemonBuild): Boolean {
        if (readOn != link) return false
        _current.value = build
        _last.value = build
        return true
    }
}
