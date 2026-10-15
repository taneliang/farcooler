package com.farcooler.account

import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.sync.Mutex
import kotlinx.serialization.json.JsonObject

/**
 * This phone's key slots, in the Keystore-wrapped store [com.farcooler.data.Identity]
 * keeps (ov-569). Only public halves come out; signing happens in the core.
 *
 * - `current` is the key the phone connects with and registers;
 * - `next` is the new key, made but not in use yet;
 * - `previous` is the old key, kept only for runners that haven't moved yet.
 */
interface PhoneKeySlots {
    val currentPublicKey: String?
    val nextPublicKey: String?
    val previousPublicKey: String?
    /** Make the new key and keep it in `next`. Null when the store refused. */
    fun makeNext(): String?
    /** `next` becomes `current`, and `current` becomes `previous`. False, with nothing moved, when it can't. */
    fun promote(): Boolean
    fun forgetPrevious()
    /** The core's answer for the relay's challenge, signed by the `next` key. */
    fun proveNext(account: String, challenge: String): String?
    /** The core's answer for a certificate, signed by the `current` key, for [keyA]. */
    fun certifyByCurrent(account: String, keyA: String): String?
    /** The runners (by id) that still hold only the old key. */
    var runnersOnPrevious: Set<String>
}

/** One of the runners saved on this phone. */
data class RekeyRunner(val id: String, val name: String)

/** How moving one runner to the new key ended. */
enum class RunnerMove(val detail: String) {
    MOVED("Moved to the new key."),
    NOT_REACHED("Moves the next time this phone reaches it."),
    NEEDS_UPDATE("Needs a newer Far Cooler. Update it, then open it on this phone."),
    REFUSED("Didn’t take the new key. Add this phone to it again from a device that’s already on it."),
    WAITING("Waiting to move."),
}

/** Which of this phone's keys reaches a runner during a re-key. */
enum class PhoneKeyChoice {
    CURRENT, PREVIOUS;

    /**
     * The other key to try once after a runner refused this one, or null. A
     * runner that moved when its answer was lost refuses the old key, and one
     * that never moved refuses the new one.
     */
    fun alternate(hasPrevious: Boolean): PhoneKeyChoice? =
        if (!hasPrevious) null else if (this == CURRENT) PREVIOUS else CURRENT

    companion object {
        /** The old key for a runner that hasn't moved, while the phone has it; the current key otherwise. */
        fun forRunner(id: String, onPrevious: Set<String>, hasPrevious: Boolean): PhoneKeyChoice =
            if (hasPrevious && id in onPrevious) PREVIOUS else CURRENT
    }
}

/** Why a re-key stopped, in words for the screen. */
enum class PhoneRekeyFailure(val message: String) {
    KEY_STORE("This phone couldn’t store a new key, so nothing changed. Try again."),
    REGISTER("Couldn’t add the new key to your account. Check your connection, then try again."),
    REMOVE_OLD("Couldn’t remove the old key from your account. Check your connection, then try again."),
}

/** Where a re-key is, for the screen. */
data class PhoneRekeyStatus(
    val working: Boolean = false,
    val finished: Boolean = false,
    val failure: PhoneRekeyFailure? = null,
    /** `trusted`, `pending` or `certified` once the new key registered. */
    val registered: String? = null,
    val needsScan: Boolean = false,
    val moves: Map<String, RunnerMove> = emptyMap(),
    val oldRemoved: Boolean = false,
)

/**
 * "Make a new key" on a phone (ov-569), the same steps as AgentKit's
 * `PhoneRekey.swift`:
 *
 * 1. Make a new key, keeping the old one until the swap is done.
 * 2. Register it ([DeviceKeyRegistration]). It's pending unless R-75's reset trusts it.
 * 3. Confirm it: the old key certifies it ([DeviceCertification]) when the
 *    directory still has the old key confirmed. Otherwise the person scans
 *    this phone's code, which shows the new key once it's in use, from a
 *    confirmed device.
 * 4. Use it, and move each runner's line to it with `client.rekey` ([move]).
 *    A runner the phone can't reach moves later; the old key is kept for it
 *    alone.
 * 5. Remove the old key at the relay, naming the new one as its successor
 *    when the old one confirmed it.
 *
 * Each step resumes from the key slots. The new key is always the phone's
 * own; its rows in `me` are found by the keys this phone holds
 * ([DeviceKeyRow.isHeldBy]), never by label or id.
 */
class PhoneRekey(
    private val keys: PhoneKeySlots,
    private val post: suspend (path: String, body: JsonObject) -> RelayAnswer,
    /** The account signed in here, or "" for nobody: then only runners move. */
    private val signedIn: () -> String,
    private val label: () -> String,
    private val runners: () -> List<RekeyRunner>,
    /** Move one runner to the current key, over a connection the old key opened. */
    var move: suspend (RekeyRunner) -> RunnerMove,
) {
    private val _status = MutableStateFlow(PhoneRekeyStatus())
    val status: StateFlow<PhoneRekeyStatus> = _status.asStateFlow()
    private val running = Mutex()

    /** Started and not finished: a new key not in use yet, or an old one kept for some runner. */
    val isUnderway: Boolean get() = keys.nextPublicKey != null || keys.previousPublicKey != null

    /** Every runner saved on this phone. */
    val allRunners: List<RekeyRunner> get() = runners()

    /** The runners still on the old key, in the order the phone lists them. */
    val runnersLeft: List<RekeyRunner> get() = runners().filter { it.id in keys.runnersOnPrevious }

    /** Start, or carry on from wherever the slots say it got to. */
    suspend fun run() {
        if (!running.tryLock()) return
        try {
            _status.update { it.copy(working = true, finished = false, failure = null) }
            val failure = steps()
            _status.update { it.copy(working = false, finished = failure == null, failure = failure) }
        } finally {
            running.unlock()
        }
    }

    /** Move one runner, after the phone reached it with the old key. */
    suspend fun moveIfNeeded(runnerId: String) {
        if (runnerId !in keys.runnersOnPrevious || keys.previousPublicKey == null) return
        val runner = runners().firstOrNull { it.id == runnerId } ?: return
        moveOne(runner)
        forgetPreviousIfDone()
    }

    /** Give up on the runners left on the old key, and delete it. */
    fun forgetOldKey() {
        keys.runnersOnPrevious = emptySet()
        keys.forgetPrevious()
    }

    private suspend fun steps(): PhoneRekeyFailure? {
        if (keys.previousPublicKey == null && keys.nextPublicKey == null) {
            keys.makeNext() ?: return PhoneRekeyFailure.KEY_STORE
        }
        if (keys.nextPublicKey != null) {
            registerAndConfirm()?.let { return it }
            // Every runner is marked BEFORE the new key is in use (fix round
            // 1, M2): stopping in between must never leave the old key with no
            // runner marked, which would read as every runner moved and delete it.
            keys.runnersOnPrevious = keys.runnersOnPrevious + runners().map { it.id }
            if (!keys.promote()) return PhoneRekeyFailure.KEY_STORE
        }
        for (runner in runners()) if (runner.id in keys.runnersOnPrevious) moveOne(runner)
        removeOld()?.let { return it }
        forgetPreviousIfDone()
        return null
    }

    private suspend fun registerAndConfirm(): PhoneRekeyFailure? {
        val account = signedIn().ifEmpty { return null }
        val registration = DeviceKeyRegistration(post, sign = keys::proveNext, label = label)
        val state = when (val outcome = registration.registerAgain(account)) {
            is DeviceKeyRegistration.Outcome.Registered -> outcome.state
            else -> return PhoneRekeyFailure.REGISTER
        }
        _status.update { it.copy(registered = state) }
        if (state != "pending") return null
        // The old key confirms the new one, if the directory still has it confirmed.
        val rows = DeviceKeys(post).list().orEmpty()
        val old = rows.firstOrNull { it.isHeldBy(keys.currentPublicKey) }
        val next = keys.nextPublicKey
        if (old != null && old.isConfirmed && next != null) {
            val certification = DeviceCertification(
                post, sign = keys::certifyByCurrent, signedIn = signedIn, waits = emptyList(), log = {},
            )
            val outcome = certification.certifyScanned(account, next)
            if (outcome == DeviceCertification.Outcome.CERTIFIED || outcome == DeviceCertification.Outcome.ALREADY_CONFIRMED) {
                _status.update { it.copy(registered = "certified") }
                return null
            }
        }
        _status.update { it.copy(needsScan = true) }
        return null
    }

    private suspend fun moveOne(runner: RekeyRunner) {
        _status.update { it.copy(moves = it.moves + (runner.id to RunnerMove.WAITING)) }
        val outcome = move(runner)
        _status.update { it.copy(moves = it.moves + (runner.id to outcome)) }
        if (outcome == RunnerMove.MOVED) keys.runnersOnPrevious = keys.runnersOnPrevious - runner.id
    }

    private suspend fun removeOld(): PhoneRekeyFailure? {
        val previous = keys.previousPublicKey ?: return null
        if (signedIn().isEmpty()) return null
        val directory = DeviceKeys(post)
        val rows = directory.list(includingRemoved = true) ?: return PhoneRekeyFailure.REMOVE_OLD
        val new = rows.firstOrNull { it.isHeldBy(keys.currentPublicKey) }
        if (new?.isConfirmed == true) _status.update { it.copy(needsScan = false) }
        val old = rows.firstOrNull { it.isHeldBy(previous) }
        if (old == null || old.state == "revoked") {
            _status.update { it.copy(oldRemoved = true) }
            return null
        }
        // The new key takes the old one's place only if the old one confirmed it.
        val successor = new?.takeIf { it.certifiedBy == old.id && it.state == "certified" }?.id
        if (!directory.remove(old.id, successor)) return PhoneRekeyFailure.REMOVE_OLD
        _status.update { it.copy(oldRemoved = true) }
        return null
    }

    private fun forgetPreviousIfDone() {
        if (keys.runnersOnPrevious.isEmpty() && keys.previousPublicKey != null) keys.forgetPrevious()
    }

    companion object {
        /** What the section says before a re-key, given whether the account still lists this phone's key. */
        fun invitation(removed: Boolean): String = if (removed) {
            "This phone was removed from your account, so your team admins can’t add it to their runners. " +
                "Make a new key to add it back."
        } else {
            "This phone signs in to your runners with its own key. Make a new one if this phone was removed " +
                "from your account, or if you think its key was copied."
        }

        const val CONFIRMATION =
            "Far Cooler makes a new key, adds it to your account and moves each of your runners to it. " +
                "Your old key stops working once that’s done. Keep Far Cooler open until it finishes."

        const val FORGET_WARNING =
            "Runners that haven’t moved still let this phone in with its old key. If you forget it, add this " +
                "phone to them again from a device that’s already on them."

        /** What the account step says, in plain words. */
        fun accountLine(signedIn: Boolean, status: PhoneRekeyStatus): String = when {
            !signedIn -> "Sign in to add the new key to your account."
            status.registered == null -> "Adding it to your account…"
            status.registered == "trusted" || status.registered == "certified" -> "Added to your account and confirmed."
            status.needsScan ->
                "Added, not confirmed yet. Show this phone’s code and scan it from one of your confirmed devices."
            else -> "Added, not confirmed yet."
        }
    }
}
