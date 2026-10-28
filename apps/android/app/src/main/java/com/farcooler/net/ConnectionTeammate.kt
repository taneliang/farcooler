package com.farcooler.net

import com.farcooler.ceremony.TeammateAddOutcome
import com.farcooler.ceremony.TeammateAddRequest
import com.farcooler.ceremony.TeammateChoice
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive

/**
 * Add a teammate's device to this runner (ov-854): its key written
 * `for_teammate` at the scope chosen (never a shell line), bound to their
 * member, and taken off again if either step fails. One call, because
 * `crates/client/src/session/teammate.rs` holds the order once for both phones;
 * this sends the words and reads the answer.
 *
 * Asked of a runner this phone administers only ([mayAddTeammate]): binding a
 * member and taking a key off are `host_admin`, and the client core answers
 * `needs_admin` without sending anything otherwise.
 */
suspend fun Connection.enrollTeammate(
    publicKey: String,
    label: String,
    clientId: String,
    nodeKey: String,
    teammate: TeammateChoice,
): TeammateAddOutcome {
    val answer = attempt {
        core.call(
            TeammateAddRequest.METHOD,
            JsonObject(
                TeammateAddRequest.arguments(publicKey, label, clientId, nodeKey, teammate)
                    .mapValues { JsonPrimitive(it.value) },
            ),
        )
    }.getOrNull()
    return TeammateAddOutcome.decode(answer)
}

/** Whether this phone holds the scope an Add Teammate needs on this runner (ov-854). */
fun Connection.mayAddTeammate(): Boolean = daemon.value?.mayAdministerRunner() == true
