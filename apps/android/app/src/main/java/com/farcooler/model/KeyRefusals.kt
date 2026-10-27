package com.farcooler.model

import com.farcooler.net.Connection

/**
 * Whether a runner that refuses this phone's key has taken its access away (ov-818), or only refused it for a moment.
 * AgentKit's `KeyRefusals`, which says why at length; the same rule on both phones.
 *
 * Since ov-783, removing a member, or taking a device off a member's list, revokes the device's key on the runner, and
 * its next dial is refused `key_rejected`. Read only against this phone's own history with the runner: whether the
 * runner has let it in before ([admitted], kept across launches by [AdmittedRunners]), and how many dials in a row it
 * has refused since. The runner says nothing new, since sshd refuses a revoked key in the words it refuses any key
 * it has never seen, so a stranger dialing it learns nothing.
 *
 * - Never let in: [Verdict.STRANGER], "Not authorized yet" as before.
 * - Let in, then refused [SETTLES_AFTER] dials in a row over [SETTLES_OVER_MS] at least: [Verdict.REVOKED].
 * - Short of that: [Verdict.PASSING], retried on the backoff, since a refusal can pass.
 *
 * Any dial that gets in starts again. A failure that isn't a refusal neither counts nor clears one.
 */
class KeyRefusals(admitted: Boolean) {
    enum class Verdict { STRANGER, PASSING, REVOKED }

    var admitted: Boolean = admitted
        private set
    var refusals: Int = 0
        private set
    private var firstAt: Long? = null
    private var lastAt: Long? = null

    /**
     * The runner said it took this phone's access away (ov-820): `released`, its link's last notice before the
     * runner closed it. A phone that reaches the runner only through Tailcat is never refused after that, only
     * ignored, so every later dial ends `no_answer`; while this holds, [reading] makes any failure to reach the
     * runner [Connection.Failure.ACCESS_REMOVED]. Kept across launches by [AdmittedRunners] until a dial gets in.
     */
    var released: Boolean = false
        private set

    /** The runner said so. */
    fun release() {
        released = true
    }

    /** A dial got in. */
    fun letIn() {
        admitted = true
        released = false
        refusals = 0
        firstAt = null
        lastAt = null
    }

    /** A dial was refused at [nowMs]: what that means now. */
    fun refused(nowMs: Long): Verdict {
        if (!admitted) return Verdict.STRANGER
        refusals += 1
        if (firstAt == null) firstAt = nowMs
        lastAt = nowMs
        return verdict
    }

    val verdict: Verdict
        get() {
            if (!admitted) return Verdict.STRANGER
            val first = firstAt ?: return Verdict.PASSING
            val last = lastAt ?: return Verdict.PASSING
            return if (refusals >= SETTLES_AFTER && last - first >= SETTLES_OVER_MS) Verdict.REVOKED else Verdict.PASSING
        }

    /** When this phone last dialed the runner (ov-821), or null. See [holdsOffDial]. */
    var lastDialMs: Long? = null
        private set

    /** A dial is starting at [nowMs]. */
    fun dialing(nowMs: Long) {
        lastDialMs = nowMs
    }

    /**
     * Whether a dial nobody asked for (the app coming forward, the network returning) should wait (ov-821): this
     * phone's access is gone ([Verdict.REVOKED], or the runner said it released it) and it dialed less than
     * [RECHECKS_EVERY_MS] ago. Each such dial is one more failed login on the runner, and a failed login costs
     * the whole address's standing, so a settled phone dials once per recheck. A person's tap and the recheck
     * itself don't ask. AgentKit's `KeyRefusals.holdsOffDial(at:)`.
     */
    fun holdsOffDial(nowMs: Long): Boolean {
        val last = lastDialMs ?: return false
        return (verdict == Verdict.REVOKED || released) && nowMs - last < RECHECKS_EVERY_MS
    }

    /**
     * Whether a refused dial should try the phone's other key too (ov-821): not once its access is gone, when the
     * second key is a second refusal on the same recheck.
     */
    val triesTheOtherKey: Boolean get() = verdict != Verdict.REVOKED && !released

    companion object {
        const val SETTLES_AFTER = 3
        const val SETTLES_OVER_MS = 10_000L

        /** How often a settled phone checks again: five minutes. */
        const val RECHECKS_EVERY_MS = 300_000L
    }
}

/**
 * A failed dial's kind, read against this phone's history with the runner (ov-818): a [Connection.Failure.KEY_REJECTED]
 * from a runner that let it in before is [Connection.Failure.REFUSED_THIS_TIME] until the refusals hold, then
 * [Connection.Failure.ACCESS_REMOVED]. Every other kind is itself. Counts the refusal into [refusals].
 */
fun Connection.Failure.reading(refusals: KeyRefusals, nowMs: Long): Connection.Failure {
    if (refusals.released && this in GONE) return Connection.Failure.ACCESS_REMOVED
    if (this != Connection.Failure.KEY_REJECTED) return this
    return when (refusals.refused(nowMs)) {
        KeyRefusals.Verdict.STRANGER -> Connection.Failure.KEY_REJECTED
        KeyRefusals.Verdict.PASSING -> Connection.Failure.REFUSED_THIS_TIME
        KeyRefusals.Verdict.REVOKED -> Connection.Failure.ACCESS_REMOVED
    }
}

/**
 * The failures a released phone reads as the release (ov-820): failing to reach the runner or be let in by it. Not
 * this phone's own trouble (no key, no node key, no tunnel in this build, a person who stopped or didn't trust a
 * key), its own network's ([Connection.Failure.TUNNEL_RENDEZVOUS]), or a changed host key, which says something else.
 */
private val GONE = setOf(
    Connection.Failure.KEY_REJECTED, Connection.Failure.UNREACHABLE, Connection.Failure.TUNNEL_NO_ANSWER,
    Connection.Failure.TUNNEL_UNSPECIFIED, Connection.Failure.REFUSED_THIS_TIME, Connection.Failure.ACCESS_REMOVED,
    Connection.Failure.OTHER,
)

/**
 * Which runners have let this phone in (ov-818), and which said they released it since (ov-820), across launches.
 * `Settings` keeps it on a phone. [remember] is a dial getting in, which ends a release.
 */
interface AdmittedRunners {
    fun admitted(runner: String): Boolean
    fun remember(runner: String)
    fun released(runner: String): Boolean
    fun release(runner: String)
}

/** For a test, and a [Connection] built without one. */
class InMemoryAdmittedRunners : AdmittedRunners {
    private val runners = mutableSetOf<String>()
    private val releasedBy = mutableSetOf<String>()
    override fun admitted(runner: String) = runner in runners
    override fun remember(runner: String) {
        runners += runner
        releasedBy -= runner
    }
    override fun released(runner: String) = runner in releasedBy
    override fun release(runner: String) {
        releasedBy += runner
    }
}
