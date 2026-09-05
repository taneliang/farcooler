package com.farcooler.data

import android.content.Context
import android.content.SharedPreferences
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.KSerializer
import kotlinx.serialization.Serializable
import kotlinx.serialization.SerializationException
import kotlinx.serialization.descriptors.SerialDescriptor
import kotlinx.serialization.encodeToString
import kotlinx.serialization.encoding.Decoder
import kotlinx.serialization.encoding.Encoder
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import java.util.UUID

/**
 * How a granted runner is reached: an address, or the tunnel.
 *
 * One or the other and never both — two nullable fields would admit "both set"
 * and "neither set", and then something here would have to pick a winner. The
 * wire is tagged on `kind` so a third kind is additive, and an unrecognized one
 * fails the decode rather than defaulting: the core has already accepted the
 * manifest by the time this runs, so a tag this build does not know is the app
 * failing, and `Refusal.Unknown` is what says so.
 *
 * This is BOTH what a ceremony reply carries and what [Runner] persists — ONE
 * type, because the two are the same fact, and translating between two spellings
 * of it is how a token ends up in a field meant for a hostname.
 */
@Serializable(with = ReachSerializer::class)
sealed interface Reach {
    data class Direct(val host: String, val port: Int) : Reach

    data class Tailcat(val token: String) : Reach

    /**
     * The second line under a runner's name.
     *
     * Never the token: it is long, it is meaningless to a person, and it is the
     * one field here worth stealing. The user still appears, because which
     * account you log in as is the other half of what the line is for.
     */
    fun detail(user: String): String = when (this) {
        is Direct -> "$user@$host"
        is Tailcat -> "$user, through the tunnel"
    }

    /** A name for a sentence, when the granting device sent no label. */
    fun name(user: String): String = when (this) {
        is Direct -> "$user@$host"
        is Tailcat -> "a tunneled runner"
    }
}

/**
 * The wire shape of a [Reach], written by hand.
 *
 * By hand rather than through `@JsonClassDiscriminator`, which is an
 * experimental API, and through a surrogate rather than raw [kotlinx.serialization.json.JsonElement],
 * so the field names live in one declaration that the compiler checks. iOS and
 * the Mac hand-write the same two functions for the same reason: three
 * platforms agreeing about a payload by inspection is three chances to
 * disagree, so each one is written out where it can be read.
 */
object ReachSerializer : KSerializer<Reach> {
    @Serializable
    private data class Wire(
        val kind: String,
        val host: String? = null,
        val port: Int? = null,
        val token: String? = null,
    )

    override val descriptor: SerialDescriptor = Wire.serializer().descriptor

    override fun serialize(encoder: Encoder, value: Reach) {
        val wire = when (value) {
            is Reach.Direct -> Wire("direct", host = value.host, port = value.port)
            is Reach.Tailcat -> Wire("tailcat", token = value.token)
        }
        encoder.encodeSerializableValue(Wire.serializer(), wire)
    }

    override fun deserialize(decoder: Decoder): Reach {
        val wire = decoder.decodeSerializableValue(Wire.serializer())
        return when (wire.kind) {
            "direct" -> Reach.Direct(
                wire.host ?: throw SerializationException("a direct reach with no host"),
                wire.port ?: throw SerializationException("a direct reach with no port"),
            )
            "tailcat" -> Reach.Tailcat(
                wire.token ?: throw SerializationException("a tunneled reach with no token"),
            )
            // The core has already accepted the manifest by the time this runs,
            // so a kind this build does not know is the app failing rather than
            // a code being refused.
            else -> throw SerializationException("unknown reach ${wire.kind}")
        }
    }
}

/**
 * A runner this device knows how to reach.
 *
 * One `farcoolerd`: a Unix user on a host, with its own worktrees. Two entries
 * may name the same box under different users, and they share nothing — which
 * is why this is a runner rather than a machine.
 */
@Serializable(with = RunnerSerializer::class)
data class Runner(
    val id: String = UUID.randomUUID().toString(),
    val label: String,
    /**
     * An address, or the tunnel. Not an `address` and a `port`, because a
     * tunneled runner has neither — it has a token — and this app could not keep
     * one at all until this field existed.
     */
    val reach: Reach,
    val user: String,
    /**
     * The host key we have accepted — the box's key, not the runner's. Null
     * means we have never connected, and the first attempt will report the
     * fingerprint rather than trusting it.
     */
    val fingerprint: String? = null,
) {
    /**
     * The JSON the client core expects.
     *
     * A tunneled runner names its `token` and this DEVICE's node private key
     * where a direct one names a `host` and a `port`: a token is not an address,
     * and `parse_destination` refuses to hold both. [nodeKey] is per device
     * rather than per runner and never travelled in the manifest — whoever dials
     * supplies the key it already holds.
     */
    fun config(privateKey: String, nodeKey: String?): JsonObject = JsonObject(
        buildMap {
            put("user", JsonPrimitive(user))
            put("private_key", JsonPrimitive(privateKey))
            when (reach) {
                is Reach.Direct -> {
                    put("host", JsonPrimitive(reach.host))
                    put("port", JsonPrimitive(reach.port))
                }
                is Reach.Tailcat -> {
                    put("token", JsonPrimitive(reach.token))
                    // Empty rather than absent when this device holds none:
                    // absent would leave a config with neither a token's key nor
                    // a host, and the core's message would name the wrong
                    // missing thing. The caller refuses before it gets here.
                    put("node_key", JsonPrimitive(nodeKey ?: ""))
                }
            }
            fingerprint?.let { put("host_fingerprint", JsonPrimitive(it)) }
        }
    )

    /**
     * What to call this runner in a sentence.
     *
     * An address for a direct runner, so every sentence this app already wrote
     * is unchanged — the common path must not move. A tunneled runner has no
     * address, so it is named by the label somebody ticked on the device that
     * granted it.
     */
    val named: String
        get() = when (reach) {
            is Reach.Direct -> reach.host
            is Reach.Tailcat -> label.ifBlank { "this runner" }
        }

    val displayLabel: String get() = label.ifBlank { named }
}

/**
 * A [Runner] on disk, in either shape it has ever been written in.
 *
 * Written by hand because runners saved before [Runner.reach] existed carry an
 * `address` and a `port` instead, and [RunnerStore] decodes the whole list in
 * one call: an entry the generated decoder threw on would not lose one runner —
 * it would lose every runner anybody had ever added, silently, on the first
 * launch after an update. An `address` with no `reach` beside it means
 * [Reach.Direct], which is what every install on disk today holds.
 *
 * Only the new shape is written. `address` and `port` are not emitted beside
 * `reach`: a second copy of where a runner lives is a second fact that can
 * disagree with the first, which is the mistake [Identity.publicKey] exists to
 * explain.
 */
object RunnerSerializer : KSerializer<Runner> {
    @Serializable
    private data class Wire(
        val id: String? = null,
        val label: String = "",
        val reach: Reach? = null,
        val address: String? = null,
        val port: Int? = null,
        val user: String = "",
        val fingerprint: String? = null,
    )

    override val descriptor: SerialDescriptor = Wire.serializer().descriptor

    override fun serialize(encoder: Encoder, value: Runner) {
        encoder.encodeSerializableValue(
            Wire.serializer(),
            Wire(
                id = value.id,
                label = value.label,
                reach = value.reach,
                user = value.user,
                fingerprint = value.fingerprint,
            ),
        )
    }

    override fun deserialize(decoder: Decoder): Runner {
        val wire = decoder.decodeSerializableValue(Wire.serializer())
        return Runner(
            id = wire.id ?: UUID.randomUUID().toString(),
            label = wire.label,
            reach = wire.reach ?: Reach.Direct(wire.address.orEmpty(), wire.port ?: 22),
            user = wire.user,
            fingerprint = wire.fingerprint,
        )
    }
}

/**
 * Known runners. Plain preferences: none of this is secret, and the one thing
 * that is lives behind the Keystore — see [Identity].
 *
 * The preference file and its keys keep their old spelling on purpose: they
 * name slots on disk that existing installs already wrote, and renaming one
 * would silently forget every runner anybody had added.
 */
class RunnerStore(context: Context) {
    private val preferences: SharedPreferences =
        context.applicationContext.getSharedPreferences("farcooler.hosts", Context.MODE_PRIVATE)

    private val json = Json { ignoreUnknownKeys = true }

    private val _hosts = MutableStateFlow<List<Runner>>(emptyList())
    val hosts: StateFlow<List<Runner>> = _hosts.asStateFlow()

    /**
     * The runner the app opens onto.
     *
     * Persisted because the phone's home screen is the terminals on a runner
     * rather than a list of runners. Landing on whichever runner happened to be
     * first in the list would mean the app forgets where you were every time
     * you close it.
     */
    private val _selectedId = MutableStateFlow<String?>(null)
    val selectedId: StateFlow<String?> = _selectedId.asStateFlow()

    val selected: Runner? get() = _hosts.value.firstOrNull { it.id == _selectedId.value }

    init {
        _hosts.value = runCatching {
            preferences.getString(KEY_HOSTS, null)?.let { json.decodeFromString<List<Runner>>(it) }
        }.getOrNull() ?: emptyList()

        // Whatever was open last, or the first runner.
        val remembered = preferences.getString(KEY_LAST, null)
        _selectedId.value =
            _hosts.value.firstOrNull { it.id == remembered }?.id ?: _hosts.value.firstOrNull()?.id
    }

    fun select(host: Runner) {
        _selectedId.value = host.id
        preferences.edit().putString(KEY_LAST, host.id).apply()
    }

    fun add(host: Runner) {
        _hosts.value = _hosts.value + host
        // Added means wanted: a runner you just typed in is the one you want
        // to be looking at, and the app opens onto whatever is selected.
        select(host)
        save()
    }

    /**
     * Correct a runner that was typed in wrong.
     *
     * The reason this exists is that a runner you cannot connect to is a runner
     * you cannot get past — the app opens onto it — so a mistyped address used
     * to be permanent on iOS until the editor was added, and the app's own
     * screens gave no way to fix or delete it.
     *
     * Clears the pinned fingerprint when the box the pin was ABOUT changes. A
     * fingerprint is a promise about one host at one address; carrying it
     * across to a corrected address would meet the new host with a changed-key
     * warning describing a machine nobody ever trusted.
     */
    fun update(host: Runner) {
        val current = _hosts.value
        val index = current.indexOfFirst { it.id == host.id }
        if (index < 0) return
        val previous = current[index]
        // Compared on the whole reach rather than on an address and a port, so
        // a runner re-pointed from an address to the tunnel — or from one token
        // to another — drops its pin too. Those are the same event: the box on
        // the other end is no longer the box the pin was taken from.
        val edited =
            if (host.reach != previous.reach) host.copy(fingerprint = null) else host
        _hosts.value = current.toMutableList().also { it[index] = edited }
        save()
    }

    /**
     * Forget a host key we pinned, so the next connection asks about it again.
     *
     * The only honest answer to "this key is not the one recorded". Either the
     * host was rebuilt, in which case the new key is fine and someone should
     * look at its fingerprint and say so, or it is an interception, in which
     * case nothing this app offers should quietly paper over it. Both roads go
     * through the approval screen, which is where this leads.
     */
    fun forgetKey(host: Runner) {
        val current = _hosts.value
        val index = current.indexOfFirst { it.id == host.id }
        if (index < 0) return
        _hosts.value = current.toMutableList().also { it[index] = it[index].copy(fingerprint = null) }
        save()
    }

    fun remove(host: Runner) {
        _hosts.value = _hosts.value.filterNot { it.id == host.id }
        if (_selectedId.value == host.id) {
            val next = _hosts.value.firstOrNull()
            _selectedId.value = next?.id
            preferences.edit().putString(KEY_LAST, next?.id).apply()
        }
        save()
    }

    /** Record the fingerprint a user has approved. */
    fun trust(host: Runner, fingerprint: String) {
        val current = _hosts.value
        val index = current.indexOfFirst { it.id == host.id }
        if (index < 0) return
        _hosts.value =
            current.toMutableList().also { it[index] = it[index].copy(fingerprint = fingerprint) }
        save()
    }

    private fun save() {
        preferences.edit().putString(KEY_HOSTS, json.encodeToString(_hosts.value)).apply()
    }

    private companion object {
        const val KEY_HOSTS = "hosts"
        const val KEY_LAST = "hosts.last"
    }
}
