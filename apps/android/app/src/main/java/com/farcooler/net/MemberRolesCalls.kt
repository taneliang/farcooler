package com.farcooler.net

import com.farcooler.model.RunnerTeam
import kotlinx.serialization.json.JsonObject

/** The people on this runner and their roles (ov-784), or null where it didn't answer. */
suspend fun Connection.runnerTeam(): RunnerTeam? {
    val reply = attempt { core.call("member.list", JsonObject(emptyMap())) }.getOrNull() ?: return null
    return RunnerTeam.of(reply)
}

/** `member.set_role`: the people as they now are, or null when the runner didn't take it. The owner's alone. */
suspend fun Connection.setRole(handle: String, role: RunnerTeam.Role): RunnerTeam? {
    val reply = attempt { core.call("member.set_role", Connection.args("handle" to handle, "role" to role.wire)) }
        .getOrNull() ?: return null
    return RunnerTeam.of(reply)
}
