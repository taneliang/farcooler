package com.farcooler.account

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * The signed-in account's device keys, as `directory/me` lists them, and
 * removing one (ov-549 review M1, the same as `AccountDeviceKeys.swift`).
 *
 * The first device an account registers is trusted on first use, and a stolen
 * session that registers first would be that device. Each app tells the person
 * about every device added or removed ([DeviceNotice]); this is where they act
 * on it, telling rows apart by their key words and date.
 */
data class DeviceKeyRow(
    val id: String,
    /** The registering session's choice, an impostor's included: never shown alone (review M2). */
    val label: String,
    val keyA: String,
    /** `trusted`, `certified`, `pending` or `revoked`. */
    val state: String,
    /** `SHA256:…`, or "" when the relay sent none. */
    val fingerprint: String = "",
    /** Milliseconds since 1970, as the relay keeps them. */
    val createdAt: Long? = null,
    val revokedAt: Long? = null,
) {
    val isConfirmed: Boolean get() = state == "trusted" || state == "certified"

    /**
     * Whether this row is the device holding [ownKeyA], compared by the key
     * itself, never its comment, and worked out here, from the key this phone
     * holds, so the relay can't move the mark onto another row.
     */
    fun isHeldBy(ownKeyA: String?): Boolean {
        val mine = ownKeyA?.let(::blob) ?: return false
        return blob(keyA) == mine
    }

    /**
     * The label as a screen may quote it: no double quote marks, so it can't
     * close the quotes around it and go on with a sentence of its own, and at
     * most 40 characters (review M2). The relay's notices use the same rule.
     */
    val shownLabel: String
        get() {
            val plain = label.filterNot { it in QUOTES }.split(Regex("\\s+")).filter { it.isNotEmpty() }.joinToString(" ")
            return plain.take(40).trim().ifEmpty { "a device" }
        }

    /** What tells two rows with the same label apart: the key words and the day it was added. */
    fun identity(day: (Long) -> String = ::dayOf): String = listOfNotNull(
        fingerprint.takeIf { it.isNotEmpty() }?.let(com.farcooler.ceremony.CeremonyStore::abbreviated),
        createdAt?.let { "Added ${day(it)}" },
    ).joinToString(" · ")

    /**
     * What the row says under the device's name. [anyConfirmed] is whether any
     * of the account's devices is confirmed: with none, there is nothing to
     * scan its code from (review L4).
     */
    fun detail(isThisDevice: Boolean, anyConfirmed: Boolean = true): String {
        val words = when (state) {
            "trusted" -> "Confirmed as the first device on your account"
            "certified" -> "Confirmed by another of your devices"
            else -> if (anyConfirmed) {
                "Not confirmed yet. Scan its code from one of your confirmed devices."
            } else {
                "Not confirmed. None of your devices is confirmed, so none can confirm it yet. " +
                    "Ask the Far Cooler team to confirm it."
            }
        }
        return if (isThisDevice) "This device. $words" else words
    }

    private fun blob(line: String): String? = line.split(" ").filter { it.isNotEmpty() }.getOrNull(1)

    companion object {
        private val QUOTES = setOf('"', '`', '\u201C', '\u201D', '\u201E', '\u201F', '«', '»', '‹', '›')

        fun dayOf(ms: Long): String = java.text.DateFormat.getDateInstance(java.text.DateFormat.MEDIUM).format(java.util.Date(ms))

        /**
         * What the Remove dialog says, warning when [row] is the last confirmed
         * device among [rows] (review M2): removing it leaves nothing that can
         * confirm the others.
         */
        fun removalWarning(row: DeviceKeyRow, rows: List<DeviceKeyRow>): String =
            if (row.isConfirmed && rows.count { it.isConfirmed } == 1) {
                "This is the only device that can confirm your others. Removing it leaves them unconfirmed, " +
                    "and none of your devices can confirm them again. Its key can’t be added back."
            } else {
                "Its key can’t be added back, and devices it confirmed will need confirming again."
            }
    }
}

class DeviceKeys(private val post: suspend (path: String, body: JsonObject) -> RelayAnswer) {
    /**
     * The account's keys, oldest first; the removed ones only when asked for,
     * since a removal is news too ([DeviceNotice]). Null when the relay
     * couldn't say.
     */
    suspend fun list(includingRemoved: Boolean = false): List<DeviceKeyRow>? {
        val answer = post(ME, JsonObject(emptyMap())) as? RelayAnswer.Answered ?: return null
        val rows = answer.body["devices"] as? JsonArray ?: return null
        return rows.mapNotNull { element ->
            val row = runCatching { element.jsonObject }.getOrNull() ?: return@mapNotNull null
            fun text(key: String) = (row[key] as? JsonPrimitive)?.contentOrNull
            val state = text("state") ?: return@mapNotNull null
            if (state == "revoked" && !includingRemoved) return@mapNotNull null
            fun ms(key: String) = (row[key] as? JsonPrimitive)?.contentOrNull?.toDoubleOrNull()?.toLong()
            DeviceKeyRow(
                id = text("id") ?: return@mapNotNull null,
                label = text("label") ?: return@mapNotNull null,
                keyA = text("keyA") ?: return@mapNotNull null,
                state = state,
                fingerprint = text("keyAFingerprint").orEmpty(),
                createdAt = ms("createdAt"),
                revokedAt = ms("revokedAt"),
            )
        }
    }

    /**
     * Remove one for good: its key can never be registered again, and each
     * device it confirmed goes back to unconfirmed (R-52).
     */
    suspend fun remove(id: String): Boolean =
        post(REVOKE, buildJsonObject { put("device", JsonPrimitive(id)) }) is RelayAnswer.Answered

    companion object {
        const val ME = "/v1/directory/me"
        const val REVOKE = "/v1/directory/devices/revoke"
    }
}
