package com.farcooler.net

import com.farcooler.account.PhoneKeyChoice
import com.farcooler.account.PhoneKeySlots
import com.farcooler.account.RunnerMove
import com.farcooler.core.ClientCore
import com.farcooler.core.NativeClient
import com.farcooler.core.UnreachedException
import com.farcooler.core.coreIsAvailable
import com.farcooler.data.Identity
import com.farcooler.model.Capability
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject

// "Make a new key" on Android (ov-569). The steps are [com.farcooler.account.PhoneRekey];
// this file is what only the app can do: pick which key reaches a runner, and
// move a runner's line over a connection the old key opened (`client.rekey`).

/** The keys a connection can choose between, and which runners haven't moved. */
interface KeyRing {
    val current: String?
    val previous: String?
    var onPrevious: Set<String>
}

/** [Identity]'s keys. */
object IdentityKeyRing : KeyRing {
    override val current: String? get() = Identity.privateKey()
    override val previous: String? get() = Identity.previousPrivateKey
    override var onPrevious: Set<String>
        get() = Identity.runnersOnPrevious
        set(value) { Identity.runnersOnPrevious = value }
}

/**
 * Connect to runner [runnerId] with the key it gets, and if it refuses that
 * key, try the phone's other one once. A runner that moved when its answer was
 * lost refuses the old key; one that never moved refuses the new one. Either
 * way, the key that worked says where the runner is now.
 */
suspend fun connectWithEitherKey(
    runnerId: String,
    ring: KeyRing,
    connect: suspend (key: String) -> Unit,
    refused: (Exception) -> Boolean = ::refusedTheKey,
    triesTheOther: Boolean = true,
) {
    val previous = ring.previous
    val tried = PhoneKeyChoice.forRunner(runnerId, ring.onPrevious, previous != null)
    val key = (if (tried == PhoneKeyChoice.PREVIOUS) previous else ring.current)
        ?: throw IllegalStateException("no key")
    try {
        connect(key)
    } catch (e: Exception) {
        e.rethrowIfCancellation()
        val other = tried.alternate(previous != null)
        if (!refused(e) || !triesTheOther || other == null) throw e
        val otherKey = (if (other == PhoneKeyChoice.PREVIOUS) previous else ring.current) ?: throw e
        connect(otherKey)
        ring.onPrevious = if (other == PhoneKeyChoice.CURRENT) ring.onPrevious - runnerId else ring.onPrevious + runnerId
    }
}

/** Whether a connect failed because the runner doesn't know the key. */
fun refusedTheKey(error: Exception): Boolean {
    val unreached = error as? UnreachedException ?: return false
    return Connection.Failure.of(unreached.trouble, unreached.tunnel) == Connection.Failure.KEY_REJECTED
}

/**
 * Move this runner's line to the key in use, over this connection, which the
 * old key opened. `client.rekey` rewrites only the line this connection
 * authenticated as.
 */
suspend fun Connection.moveToNewKey(): RunnerMove {
    if (phase.value !is Connection.Phase.Connected) return RunnerMove.NOT_REACHED
    loadDaemonBuild()
    val build = daemon.value ?: return RunnerMove.NOT_REACHED
    if (!build.can(Capability.REKEY)) return RunnerMove.NEEDS_UPDATE
    val proof = rekeyProof() ?: return RunnerMove.REFUSED
    return try {
        core.call("client.rekey", proof)
        RunnerMove.MOVED
    } catch (e: Exception) {
        e.rethrowIfCancellation()
        // A dropped link isn't a refusal: the next connection finds out which
        // key the runner takes.
        if (phase.value is Connection.Phase.Connected) RunnerMove.REFUSED else RunnerMove.NOT_REACHED
    }
}

/** What `client.rekey` takes: the key in use, proven over the old one. */
private fun rekeyProof(): JsonObject? {
    if (!coreIsAvailable) return null
    val new = Identity.privateKey() ?: return null
    val old = Identity.previousPrivateKey?.let(ClientCore::publicKey) ?: return null
    val answer = NativeClient.nativeRekeyProof(new, old) ?: return null
    val parsed = runCatching { Json.parseToJsonElement(answer).jsonObject }.getOrNull() ?: return null
    if (parsed["error"] != null || parsed["proof"] !is JsonPrimitive) return null
    return parsed
}

/** [Identity]'s slots, as [com.farcooler.account.PhoneRekey] sees them: public halves only. */
object IdentityKeySlots : PhoneKeySlots {
    override val currentPublicKey: String? get() = Identity.publicKey
    override val nextPublicKey: String? get() = Identity.nextPrivateKey?.let(ClientCore::publicKey)
    override val previousPublicKey: String? get() = Identity.previousPrivateKey?.let(ClientCore::publicKey)
    override fun makeNext(): String? = Identity.makeNext()
    override fun promote(): Boolean = Identity.promote()
    override fun forgetPrevious() = Identity.forgetPrevious()
    override fun proveNext(account: String, challenge: String): String? =
        if (!coreIsAvailable) null
        else Identity.nextPrivateKey?.let { NativeClient.nativeDeviceRegistration(it, account, challenge) }
    override fun certifyByCurrent(account: String, keyA: String): String? =
        if (!coreIsAvailable) null
        else Identity.privateKey()?.let { NativeClient.nativeDeviceCertificate(it, account, keyA) }
    override var runnersOnPrevious: Set<String>
        get() = Identity.runnersOnPrevious
        set(value) { Identity.runnersOnPrevious = value }
}
