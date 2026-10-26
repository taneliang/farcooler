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

    /** A dial got in. */
    fun letIn() {
        admitted = true
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

    companion object {
        const val SETTLES_AFTER = 3
        const val SETTLES_OVER_MS = 10_000L
    }
}

/**
 * A failed dial's kind, read against this phone's history with the runner (ov-818): a [Connection.Failure.KEY_REJECTED]
 * from a runner that let it in before is [Connection.Failure.REFUSED_THIS_TIME] until the refusals hold, then
 * [Connection.Failure.ACCESS_REMOVED]. Every other kind is itself. Counts the refusal into [refusals].
 */
fun Connection.Failure.reading(refusals: KeyRefusals, nowMs: Long): Connection.Failure {
    if (this != Connection.Failure.KEY_REJECTED) return this
    return when (refusals.refused(nowMs)) {
        KeyRefusals.Verdict.STRANGER -> Connection.Failure.KEY_REJECTED
        KeyRefusals.Verdict.PASSING -> Connection.Failure.REFUSED_THIS_TIME
        KeyRefusals.Verdict.REVOKED -> Connection.Failure.ACCESS_REMOVED
    }
}

/** Which runners have let this phone in, across launches (ov-818). `Settings` keeps it on a phone. */
interface AdmittedRunners {
    fun admitted(runner: String): Boolean
    fun remember(runner: String)
}

/** For a test, and a [Connection] built without one. */
class InMemoryAdmittedRunners : AdmittedRunners {
    private val runners = mutableSetOf<String>()
    override fun admitted(runner: String) = runner in runners
    override fun remember(runner: String) {
        runners += runner
    }
}
