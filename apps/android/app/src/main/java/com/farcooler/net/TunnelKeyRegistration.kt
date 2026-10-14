package com.farcooler.net

import com.farcooler.data.NodeIdentity
import com.farcooler.data.Reach
import com.farcooler.data.RunnerStore
import com.farcooler.model.Capability
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonPrimitive

/**
 * A runner this phone was paired with before the tunnel existed learns this
 * phone's node key over the access it already has (ov-485,
 * `client.set_node_key`), so the fleet gains Tailcat without pairing again.
 *
 * **Kept, not switched.** The runner's token goes beside it as its other route
 * ([RunnerStore.alternatives]), not in place of the address that works now:
 * a working connection is never dropped for a route nobody has tried. The
 * runner's editor shows it, and one tap moves it.
 *
 * Once per runner and key per launch, until the runner answers with a token.
 */
class TunnelKeyRegistration(
    private val hosts: RunnerStore,
    private val nodeKey: () -> String? = { NodeIdentity.offeredPublicKey },
) {
    private val tried = mutableSetOf<String>()

    /** Called when [connection] comes up. */
    suspend fun connected(connection: Connection) {
        val runner = connection.host
        if (runner.reach !is Reach.Direct || hosts.alternatives.value[runner.id] != null) return
        // No pin, no tunnel: the tunnel route is held to the key this phone
        // already trusts (R-48), and an unpinned runner has none.
        if (runner.fingerprint?.startsWith("SHA256:") != true) return
        val key = nodeKey()?.takeIf { it.isNotEmpty() } ?: return
        if (!tried.add("${runner.id}\n$key")) return
        connection.loadDaemonBuild()
        if (connection.daemon.value?.can(Capability.TUNNEL) != true) return
        val answer = runCatching {
            connection.core.call("client.set_node_key", buildJsonObject { put("nodeKey", JsonPrimitive(key)) })
        }.getOrNull() ?: return
        val token = answer["connBlob"]?.jsonPrimitive?.contentOrNull.orEmpty()
        if (token.isNotEmpty()) hosts.keepAlternative(runner.id, Reach.Tailcat(token))
    }
}
