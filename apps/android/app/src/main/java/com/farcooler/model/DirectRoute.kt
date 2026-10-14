package com.farcooler.model

import com.farcooler.data.Reach
import com.farcooler.data.Runner
import com.farcooler.net.Connection

/**
 * Which route a runner is reached by, and what is said when the tunnel fails
 * for one that also has an address (ov-485).
 *
 * **The owner's rule, Oct 13: the tunnel first, the address only on request.**
 * A ceremony grants the tunnel and keeps the address beside it. Nothing dials
 * the address on its own: when the tunnel fails the app says so and asks, and
 * only a tap moves the runner. One route at a time, named on screen.
 *
 * The same rule and words as AgentKit's `TunnelRoute.swift`, in Android's
 * sentence case.
 */
object DirectRoute {
    /** "Connecting through Tailcat failed. Connect directly to <address> instead?" */
    data class Offer(val address: String) {
        val title: String get() = "Connecting through Tailcat failed."
        val question: String get() = "Connect directly to $address instead?"
        val sentence: String get() = "$title $question"
    }

    const val CONNECT_DIRECTLY = "Connect directly"
    const val USE_TAILCAT = "Use Tailcat"
    const val HEADING = "Route"

    /** The port only when it isn't 22, the port nobody types. */
    fun address(host: String, port: Int): String = if (port == 22) host else "$host:$port"

    /** What a person reads for the route a runner takes. */
    fun label(reach: Reach): String = when (reach) {
        is Reach.Tailcat -> "Through Tailcat"
        is Reach.Direct -> "Directly to ${address(reach.host, reach.port)}"
    }

    /** The button that moves a runner to its other route. */
    fun switchLabel(reach: Reach): String = when (reach) {
        is Reach.Tailcat -> CONNECT_DIRECTLY
        is Reach.Direct -> USE_TAILCAT
    }

    /**
     * Whether nothing on the runner answered: the failures after which its
     * address is worth offering. A refusal from sshd or the daemon would be
     * the same refusal at the address.
     */
    fun failedBeforeTheRunner(failure: Connection.Failure): Boolean = when (failure) {
        Connection.Failure.UNREACHABLE,
        Connection.Failure.NO_NODE_KEY,
        Connection.Failure.STOPPED,
        Connection.Failure.TUNNEL_NO_ANSWER,
        Connection.Failure.TUNNEL_RENDEZVOUS,
        Connection.Failure.TUNNEL_NOT_IN_THIS_BUILD,
        Connection.Failure.TUNNEL_UNSPECIFIED,
        -> true
        Connection.Failure.KEY_REJECTED,
        Connection.Failure.HOST_KEY_CHANGED,
        Connection.Failure.DAEMON_MISSING,
        Connection.Failure.NO_IDENTITY,
        Connection.Failure.KEY_NOT_TRUSTED,
        Connection.Failure.OTHER,
        -> false
    }

    /**
     * The question to ask after [trouble], or null: a runner on the tunnel, a
     * failure where nothing on it answered, and an address kept beside it.
     */
    fun offer(runner: Runner, alternative: Reach?, trouble: Connection.Failure?): Offer? {
        if (trouble == null || runner.reach !is Reach.Tailcat) return null
        if (alternative !is Reach.Direct || !failedBeforeTheRunner(trouble)) return null
        return Offer(address(alternative.host, alternative.port))
    }

    /**
     * What a phone grants for one runner it is sharing: the route the new
     * device dials, and the one kept beside it. iOS's `TunnelRoute.grant`.
     *
     * A runner that answered `client.enroll` with [token] goes through the
     * tunnel with its address beside it; one already on the tunnel stays, with
     * any address this phone keeps; a device with no node key gets the address
     * (or the tunnel, for the core to refuse, when there is none). A token this
     * phone merely holds is never granted: only a fresh one says the NEW
     * device's key is admitted.
     */
    fun grant(reach: Reach, kept: Reach?, token: String, deviceHasNodeKey: Boolean): Pair<Reach, Reach?> {
        val address = listOfNotNull(reach, kept).firstOrNull { it is Reach.Direct }
        if (!deviceHasNodeKey) {
            return if (reach is Reach.Tailcat && address != null) address to null else reach to null
        }
        if (reach is Reach.Tailcat) return reach to address
        if (token.isNotEmpty()) return Reach.Tailcat(token) to reach
        return reach to null
    }
}
