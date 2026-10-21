package com.farcooler.net

import com.farcooler.model.MemberOwnLogins
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull

/**
 * The switches for members' own logins (ov-652), with who this phone is on
 * the runner (`you`, empty for the owner), from `member.list`; null where
 * the runner didn't answer, has none, or nobody but its owner works on it,
 * as the Mac and the iPhone show them only on a shared runner.
 */
suspend fun Connection.memberOwnLogins(): Pair<MemberOwnLogins, String>? {
    val reply = attempt { core.call("member.list", JsonObject(emptyMap())) }.getOrNull() ?: return null
    val logins = MemberOwnLogins.of(reply) ?: return null
    val you = (reply["you"] as? JsonPrimitive)?.contentOrNull ?: ""
    val members = reply["members"] as? JsonArray
    if (members.isNullOrEmpty() && you.isEmpty()) return null
    return logins to you
}

/** `member.set_own_login`: the switches as they now are, or null when the runner didn't take it. */
suspend fun Connection.setOwnLogin(agent: MemberOwnLogins.Agent, on: Boolean): MemberOwnLogins? {
    val reply = attempt { core.call("member.set_own_login", Connection.args("harness" to agent.wire, "on" to on)) }
        .getOrNull() ?: return null
    return MemberOwnLogins.of(reply)
}
