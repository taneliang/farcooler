package com.farcooler.net

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import com.farcooler.core.CoreException
import com.farcooler.model.PeopleAccess
import com.farcooler.model.StepUpFailure
import com.farcooler.model.StepUpReason
import com.farcooler.model.StepUpWords
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * The People screen's Access, devices and keys for one runner (ov-856, §8): what to draw, and what each button does
 * through a step-up. AgentKit's `PeopleAccessModel`.
 *
 * Every change goes the runner's way: a phone below `host_admin` asks the person to verify once per action, and the
 * runner holds it to that exact action. The words a refusal uses are [StepUpWords]'.
 */
class PeopleAccessController(
    private val read: suspend () -> PeopleAccess?,
    private val stepUp: () -> StepUp?,
    private val runner: String,
    private val device: String = "phone",
) {
    var access by mutableStateOf<PeopleAccess?>(null)
        private set

    /** The six digits this phone shows while its own key waits. */
    var shownCode by mutableStateOf<String?>(null)
        private set

    /** A sentence for the last action that didn't happen, or null. */
    var note by mutableStateOf<String?>(null)
        private set

    suspend fun load() {
        access = read()?.takeIf { it.recorded }
    }

    private val hasTrustedKey: Boolean
        get() = access?.thisDevice?.stepUp == com.farcooler.model.StepUpKeyStanding.TRUSTED

    private suspend fun perform(method: String, args: JsonObject, reason: String): Boolean {
        val flow = stepUp()
        if (flow == null) {
            note = StepUpWords.sentence(StepUpFailure.KeyUnavailable, runner, device)
            return false
        }
        try {
            flow.perform(method, args, reason, hasTrustedKey)
            note = null
            return true
        } catch (e: StepUpFailure) {
            note = StepUpWords.sentence(e, runner, device)
        } catch (e: CoreException) {
            note = StepUpWords.sentenceForRefusal(e.what ?: e.word.orEmpty(), runner, device)
        }
        return false
    }

    suspend fun remove(target: PeopleAccess.Device, withAdded: Boolean) {
        val reason = StepUpReason.removeDevice(target.name, if (withAdded) target.added else 0, runner)
        val args = buildJsonObject {
            put("clientId", target.id)
            put("withAdded", withAdded)
        }
        if (perform("client.revoke", args, reason)) load()
    }

    suspend fun approve(target: PeopleAccess.Device) {
        val args = buildJsonObject {
            put("clientId", target.id)
            put("code", target.code)
        }
        if (perform("client.approve_step_up", args, StepUpReason.approve(target.name))) load()
    }

    /** Offer this phone's key and show the digits another of the person's devices checks. */
    suspend fun allowAdminActions() {
        val flow = stepUp()
        if (flow == null) {
            note = StepUpWords.sentence(StepUpFailure.KeyUnavailable, runner, device)
            return
        }
        try {
            shownCode = flow.register()
            note = null
            load()
        } catch (e: StepUpFailure) {
            note = StepUpWords.sentence(e, runner, device)
        } catch (e: CoreException) {
            note = StepUpWords.sentenceForRefusal(e.what ?: e.word.orEmpty(), runner, device)
        }
    }
}
