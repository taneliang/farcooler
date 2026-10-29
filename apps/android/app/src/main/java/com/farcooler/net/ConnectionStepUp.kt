package com.farcooler.net

import android.content.Context
import com.farcooler.ceremony.CeremonyCore
import com.farcooler.data.Identity
import com.farcooler.model.PeopleAccess
import kotlinx.serialization.json.JsonObject

/** This phone's one step-up key (ov-856). A test swaps it for one that answers without a face. */
object StepUpSigners {
    @Volatile
    var override: StepUpSigning? = null

    private var keystore: KeystoreStepUpSigner? = null

    @Synchronized
    fun shared(context: Context): StepUpSigning =
        override ?: keystore ?: KeystoreStepUpSigner(context.applicationContext).also { keystore = it }
}

/** The id this phone is under on its runners: derived in Rust from its key, the way its enrollment named it. */
val ownClientId: String? get() = Identity.publicKey?.let(CeremonyCore::clientId)

/**
 * Step-up for this runner, or null when the phone can't make one yet: it needs its own id and the runner's pinned host
 * key (a runner taken on trust has none to sign).
 */
fun Connection.stepUp(context: Context): StepUp? {
    val pin = host.fingerprint?.takeIf { it.startsWith("SHA256:") } ?: return null
    val id = ownClientId ?: return null
    return StepUp(CoreStepUpCalls { method, args -> core.call(method, args) }, StepUpSigners.shared(context), id, pin)
}

/** Who is here and what each may do, with the devices under them: `member.list` and `client.list`. */
suspend fun Connection.peopleAccess(): PeopleAccess? {
    val members = attempt { core.call("member.list", JsonObject(emptyMap())) }.getOrNull() ?: return null
    val clients = attempt { core.call("client.list", JsonObject(emptyMap())) }.getOrNull()
    return PeopleAccess.of(members, clients, ownClientId.orEmpty())
}
