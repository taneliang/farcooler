package com.farcooler.model

/** What a step-up can come to besides an answer (ov-856). AgentKit's `StepUpError`. */
sealed class StepUpFailure(message: String) : Exception(message) {
    /** The person closed the prompt. Nothing to say. */
    object Cancelled : StepUpFailure("cancelled")

    /** This device has no screen lock, so it can't hold a key that needs one. */
    object NoScreenLock : StepUpFailure("no screen lock")

    /** The device can't make or use the key. */
    object KeyUnavailable : StepUpFailure("key unavailable")

    /** The runner refused, by its word. */
    class Refused(val word: String) : StepUpFailure(word)
}

/** What a prompt says, per action (§8 of the design): AgentKit's `StepUpReason`. */
object StepUpReason {
    fun remove(person: String, runner: String) = "Remove $person from $runner."

    fun removeDevice(device: String, alsoRemoving: Int, runner: String) = when (alsoRemoving) {
        0 -> "Remove $device from $runner."
        1 -> "Remove $device and the device it added."
        else -> "Remove $device and the $alsoRemoving devices it added."
    }

    fun add(device: String, runner: String, access: PersonAccess) = "Add $device to $runner as ${article(access.title)} ${access.title}."

    fun approve(device: String) = "Allow $device to approve changes to who has access."

    private fun article(word: String) = if (word.first() in "AEIOU") "an" else "a"
}

/** What a refusal says, in sentences with no runner error in them. AgentKit's `StepUpWords`. */
object StepUpWords {
    /** What [failure] means to the person, or null when they closed the prompt themselves. */
    fun sentence(failure: StepUpFailure, runner: String, device: String): String? = when (failure) {
        StepUpFailure.Cancelled -> null
        StepUpFailure.NoScreenLock -> "Set a screen lock on this $device to allow admin actions."
        StepUpFailure.KeyUnavailable -> "This $device can’t keep an admin key. Use another device."
        is StepUpFailure.Refused -> sentenceForRefusal(failure.word, runner, device)
    }

    fun sentenceForRefusal(word: String, runner: String, device: String): String = when (word) {
        "step_up_needed", "key_untrusted", "key_untrusted_here" ->
            "Allow admin actions on this $device from your Mac first."
        "step_up_unneeded", "step_up_unavailable", "step_up" ->
            "Update Far Cooler on $runner to change access here."
        "challenge_expired" -> "That took too long. Try again."
        "step_up_failed", "host_pin_mismatch", "step_up_replayed" -> "Couldn’t confirm it was you. Try again."
        "defensive_only", "not_admin", "not_yours", "socket_only", "person_only" -> "You can’t change that from this $device."
        "wrong_code" -> "That code doesn’t match. Check the other device and try again."
        "proposal_expired" -> "That request timed out. Ask again on the other device."
        else -> "Couldn’t do that. Try again."
    }
}

/**
 * The app lock (ov-856, §3a): a screen-lock prompt when the app resumes after five minutes away, before any terminal
 * or admin screen. Pure state: the clock is passed in, so a test goes five minutes in no time. AgentKit's `AppLock`.
 */
class AppLock(var isOn: Boolean = true, locked: Boolean = false) {
    var isLocked: Boolean = locked && isOn
        private set
    private var leftAt: Long? = null

    fun didLeave(nowMs: Long) {
        leftAt = nowMs
    }

    /** The app is back: locks when it was away long enough. A resume with no record of leaving locks too. */
    fun didReturn(nowMs: Long) {
        if (!isOn) return
        val left = leftAt
        if (left == null || nowMs - left >= IDLE_LIMIT_MS) isLocked = true
        leftAt = null
    }

    fun unlock() {
        isLocked = false
    }

    enum class Screen { TERMINAL, ADMIN, OTHER }

    /** Whether [screen] is covered right now. */
    fun covers(screen: Screen): Boolean = isOn && isLocked && screen != Screen.OTHER

    companion object {
        const val IDLE_LIMIT_MS = 5 * 60 * 1000L
        const val PROMPT = "Unlock Far Cooler to use your terminals and change who has access."
        const val UNLOCK_BUTTON = "Unlock"

        /** A fresh launch: locked when it's on, because nothing has verified the person yet. */
        fun atLaunch(isOn: Boolean = true) = AppLock(isOn, locked = true)
    }
}
