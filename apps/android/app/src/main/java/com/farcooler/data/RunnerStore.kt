package com.farcooler.data

import android.content.Context
import android.content.SharedPreferences
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.serialization.Serializable
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import java.util.UUID

/**
 * A runner this device knows how to reach.
 *
 * One `farcoolerd`: a Unix user on a host, with its own worktrees. Two entries
 * may name the same box under different users, and they share nothing — which
 * is why this is a runner rather than a machine.
 */
@Serializable
data class Runner(
    val id: String = UUID.randomUUID().toString(),
    val label: String,
    val address: String,
    val port: Int = 22,
    val user: String,
    /**
     * The host key we have accepted — the box's key, not the runner's. Null
     * means we have never connected, and the first attempt will report the
     * fingerprint rather than trusting it.
     */
    val fingerprint: String? = null,
) {
    /** The JSON the client core expects. */
    fun config(privateKey: String): JsonObject = JsonObject(
        buildMap {
            put("host", JsonPrimitive(address))
            put("port", JsonPrimitive(port))
            put("user", JsonPrimitive(user))
            put("private_key", JsonPrimitive(privateKey))
            fingerprint?.let { put("host_fingerprint", JsonPrimitive(it)) }
        }
    )

    val displayLabel: String get() = label.ifBlank { address }
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
        val edited =
            if (host.address != previous.address || host.port != previous.port) {
                host.copy(fingerprint = null)
            } else {
                host
            }
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
