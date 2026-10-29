package com.farcooler.net

import android.content.Context
import com.farcooler.ceremony.TeammateAccessRules
import com.farcooler.ceremony.TeammateAddOutcome
import com.farcooler.ceremony.TeammateChoice
import com.farcooler.ceremony.TeammateRunnerFacts
import com.farcooler.core.CoreException
import com.farcooler.model.Capability
import com.farcooler.model.StepUpFailure
import com.farcooler.model.StepUpWords
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject

/**
 * What this phone is on this runner and who is there already (ov-856): what Add Teammate offers, and whether the runner
 * is asked at all. Null when the runner can't be asked or doesn't record the people's access.
 */
suspend fun Connection.teammateFacts(account: String): TeammateRunnerFacts? {
    if (daemon.value?.can(Capability.PERSON_ACCESS) != true) return null
    val members = attempt { core.call("member.list", JsonObject(emptyMap())) }.getOrNull() ?: return null
    return TeammateRunnerFacts.of(members, account, ownClientId.orEmpty())
}

/**
 * Add a teammate's device to this runner (ov-856): one request that writes the line `for_teammate` at the access's
 * scope and binds it to the person, in one write, signed with a prompt for exactly that person, access and device. The
 * runner holds the rules: only an Owner makes an Admin.
 */
suspend fun Connection.enrollTeammate(
    context: Context,
    publicKey: String,
    label: String,
    clientId: String,
    nodeKey: String,
    teammate: TeammateChoice,
    facts: TeammateRunnerFacts?,
): TeammateAddOutcome {
    val runner = host.label
    val device = "phone"
    val args = TeammateAccessRules.arguments(publicKey, label, clientId, nodeKey, teammate, facts)
    val access = (args["teammate"]?.jsonObject?.get("access") as? JsonPrimitive)?.contentOrNull ?: "member"
    val flow = stepUp(context)
        ?: return TeammateAddOutcome.Declined(StepUpWords.sentence(StepUpFailure.KeyUnavailable, runner, device).orEmpty())
    return try {
        val answer = flow.perform("client.enroll", args, TeammateAccessRules.reason(label, runner, access), facts?.signs == true)
        TeammateAddOutcome.Written((answer["connBlob"] as? JsonPrimitive)?.contentOrNull.orEmpty())
    } catch (e: StepUpFailure) {
        TeammateAddOutcome.Declined(StepUpWords.sentence(e, runner, device).orEmpty())
    } catch (e: CoreException) {
        TeammateAddOutcome.Declined(TeammateAccessRules.refusal(e.what ?: e.word.orEmpty(), runner, device))
    }
}
