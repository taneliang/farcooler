package com.farcooler.net

import com.farcooler.model.EnrolledDevices
import kotlinx.serialization.json.JsonObject

/** `client.list`: the devices that may log in to this runner, or null where it didn't answer this phone. */
suspend fun Connection.enrolledDevices(): EnrolledDevices? {
    val reply = attempt { core.call("client.list", JsonObject(emptyMap())) }.getOrNull() ?: return null
    return EnrolledDevices.of(reply)
}

/** The arguments of `client.revoke`: `withAdded` only for the choice that takes what the device added too (ov-855). */
fun revokeArgs(clientId: String, withAdded: Boolean): JsonObject =
    if (withAdded) Connection.args("clientId" to clientId, "withAdded" to true) else Connection.args("clientId" to clientId)

/** `client.revoke`: the list as it now stands, or null when the runner didn't take it. */
suspend fun Connection.revokeDevice(clientId: String, withAdded: Boolean): EnrolledDevices? {
    val reply = attempt { core.call("client.revoke", revokeArgs(clientId, withAdded)) }.getOrNull() ?: return null
    return EnrolledDevices.of(reply)
}
