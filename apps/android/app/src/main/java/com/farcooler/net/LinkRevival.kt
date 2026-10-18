package com.farcooler.net

import com.farcooler.core.CoreException
import com.farcooler.core.DisconnectedException
import com.farcooler.core.UnreachedException
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Deferred
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * What the app does about its link to a runner when it comes back to the
 * foreground, and how a new link is brought up (ov-613).
 *
 * The iPhone's `LinkRevival` in AgentKit, with the same rules:
 *
 * - A link that says it is up is probed on a 2.5 s clock, alongside the fleet
 *   read that repaints. Before, the check was an ordinary `fleet` read with a
 *   30 s deadline whose timeout nothing acted on, so a link that died silently
 *   while the app was away looked alive until the ssh keepalive noticed, 90 to
 *   120 s later.
 * - A dead verdict, or a drop the fleet read finds meanwhile, dials at once.
 *   The backoff starts only when that dial fails.
 * - A dial that set out before the app went away, and has had the probe's
 *   deadline, is started again. One begun since is left however slow, so a
 *   person switching apps can't keep a first connect from ever finishing.
 * - A new link's reads go out together ([together]), and the boards are swept
 *   once they are in.
 */
object LinkRevival {
    /** `deadlines::PROBE` in `crates/client/src/deadlines.rs`, as this side's backstop. */
    const val PROBE_DEADLINE_MS = 2_500L

    /** Where the link stood when the app came back. */
    sealed interface Standing {
        data object Connected : Standing
        /** A dial in flight, for how long, and whether it began before the app went away. */
        data class Dialing(val forMs: Long, val beganBeforeLeaving: Boolean) : Standing
        /** Waiting out a backoff, or stopped on a failure, with no dial out. */
        data object Retrying : Standing
        /** A host key waiting on a person. */
        data object WaitingOnPerson : Standing
    }

    /** What the probe found. */
    enum class Probe {
        /** The runner spoke, even to refuse: the link is up. */
        ANSWERED,
        /** Nothing by the deadline. */
        SILENT,
        /** The core says the link is gone. */
        DROPPED;

        val linkIsDead: Boolean get() = this != ANSWERED
    }

    /** What coming back did, for the log. */
    sealed interface Outcome {
        data object Alive : Outcome
        data class Redialed(val probe: Probe) : Outcome
        data object RestartedDial : Outcome
        data object Dialed : Outcome
        data object Left : Outcome
    }

    /**
     * The app came back. Decide, probe at most once, and dial or refresh.
     * [refresh] runs on in [scope] after this returns with the verdict.
     */
    suspend fun onReturn(
        standing: Standing,
        scope: CoroutineScope,
        probe: suspend () -> Probe,
        dialNow: (Outcome) -> Unit,
        refresh: suspend () -> Unit,
    ): Outcome = when (standing) {
        Standing.WaitingOnPerson -> Outcome.Left
        Standing.Retrying -> Outcome.Dialed.also(dialNow)
        is Standing.Dialing ->
            if (standing.beganBeforeLeaving && standing.forMs >= PROBE_DEADLINE_MS) {
                Outcome.RestartedDial.also(dialNow)
            } else {
                Outcome.Left
            }
        Standing.Connected -> {
            scope.launch { refresh() }
            val found = probe(PROBE_DEADLINE_MS, probe)
            if (found.linkIsDead) Outcome.Redialed(found).also(dialNow) else Outcome.Alive
        }
    }

    /** Ask, and take silence by [deadlineMs] as the answer. */
    suspend fun probe(deadlineMs: Long, ask: suspend () -> Probe): Probe =
        withTimeoutOrNull(deadlineMs) { ask() } ?: Probe.SILENT

    /** A call's outcome as a verdict: a refusal is still the runner speaking. */
    suspend fun verdict(call: suspend () -> Unit): Probe = try {
        call()
        Probe.ANSWERED
    } catch (e: DisconnectedException) {
        Probe.DROPPED
    } catch (e: UnreachedException) {
        Probe.DROPPED
    } catch (e: CoreException) {
        Probe.ANSWERED
    }

    /** What a drop the fleet read found does: at once while coming back, else the backoff's first rung. */
    sealed interface AfterDrop {
        data object DialNow : AfterDrop
        data class Backoff(val attempt: Int) : AfterDrop
    }

    fun afterDrop(returning: Boolean): AfterDrop = if (returning) AfterDrop.DialNow else AfterDrop.Backoff(1)

    /**
     * How long to wait before the next attempt.
     *
     * The same schedule as the Mac's `DaemonClient.backoffSeconds` and the
     * iPhone's `Connection.backoff`, deliberately: "how long until it comes
     * back" should have one answer across the three apps. Doubling from two
     * seconds to a thirty second ceiling, with jitter, because several
     * runners recovering from one network event must not retry in
     * lockstep — and this app connects to every runner at once, so that is
     * the ordinary case here rather than the unlucky one.
     */
    fun backoffMs(attempt: Int): Long {
        val ceiling = minOf(30.0, Math.pow(2.0, attempt.toDouble()))
        return (ceiling * (0.8 + Math.random() * 0.4) * 1000).toLong()
    }

    /** Run [reads] all at once, and return when every one has. */
    suspend fun together(vararg reads: suspend () -> Unit) = coroutineScope {
        reads.forEach { read -> launch { read() } }
    }
}

/**
 * One read however many callers are waiting on it: the `host` read every
 * read that needs the build shares, so a bring-up and the fleet read beside it
 * don't each ask.
 */
class SharedRead(private val scope: CoroutineScope, private val read: suspend () -> Unit) {
    private var running: Deferred<Unit>? = null

    suspend fun await() {
        val job = synchronized(this) { running?.takeIf { it.isActive } ?: scope.async { read() }.also { running = it } }
        job.await()
    }
}

/**
 * Where a connection's link stands for a return, and the step log that times
 * it (ov-613): `adb logcat -s FarCoolerLink`. Each line starts with the
 * connection's ordinal; step names and milliseconds only, never a runner's
 * name, an address or an error's text.
 */
class LinkSteps(
    private val now: () -> Long = { android.os.SystemClock.elapsedRealtime() },
    private val sink: (String) -> Unit = { runCatching { android.util.Log.i("FarCoolerLink", it) } },
) {
    private val ordinal = issued.incrementAndGet()
    /** When the dial in flight began, or null with none out. */
    @Volatile var dialSince: Long? = null
        private set
    @Volatile private var leftAt: Long? = null
    @Volatile private var lastDialBegan: Long? = null
    @Volatile private var returnedAt: Long? = null
    /** Returns still vouching for their link: a count, as two quick returns overlap. */
    val returns = AtomicInteger()

    fun left() { leftAt = now() }

    fun dialBegan(): Long = now().also { dialSince = it; log("dial") }

    fun dialEnded(began: Long, connected: Boolean) {
        log("${if (connected) "connected" else "dial failed"} in ${now() - began} ms")
        if (dialSince == began) dialSince = null
        if (connected) lastDialBegan = began
    }

    /** What [phase] means to a return. */
    fun standing(phase: Connection.Phase): LinkRevival.Standing = when (phase) {
        is Connection.Phase.Connected -> LinkRevival.Standing.Connected
        is Connection.Phase.Connecting, is Connection.Phase.Reconnecting -> when (val began = dialSince) {
            null -> if (phase is Connection.Phase.Connecting) LinkRevival.Standing.Dialing(0, false) else LinkRevival.Standing.Retrying
            else -> LinkRevival.Standing.Dialing(now() - began, leftAt?.let { began < it } ?: false)
        }
        is Connection.Phase.Failed -> LinkRevival.Standing.Retrying
        is Connection.Phase.NeedsApproval -> LinkRevival.Standing.WaitingOnPerson
    }

    /** The probe, timed in the log. */
    suspend fun probe(ask: suspend () -> LinkRevival.Probe): LinkRevival.Probe {
        val began = now()
        return ask().also { log("probe: $it in ${now() - began} ms") }
    }

    fun cameBack(standing: LinkRevival.Standing) {
        returnedAt = now()
        log("foreground: $standing")
    }

    /** A fleet landed on the link that is up: the first on it, or not. */
    fun fleetLanded(first: Boolean) {
        lastDialBegan?.takeIf { first }?.let { log("first fleet in ${now() - it} ms from dial") }
        returnedAt?.let { log("live in ${now() - it} ms from foreground") }
        returnedAt = null
    }

    fun log(step: String) = sink("[$ordinal] $step")

    private companion object {
        val issued = AtomicInteger()
    }
}
