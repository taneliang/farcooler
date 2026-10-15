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
    /**
     * When it was trusted on first use: its arrival for an account's first
     * device, later for one trusted again because none of the account's
     * devices was confirmed any more (ruling R-75, ov-561).
     */
    val trustedAt: Long? = null,
    /**
     * Whether that trust was R-75's reset rather than the account's first
     * device: a new device trusted with none of the others confirmed, or one
     * already on the account trusted when it signed in again.
     */
    val trustReset: Boolean = false,
) {
    /** Trusted by R-75's reset, as the relay says, however it arrived: never "the first device". */
    val wasTrustedAgain: Boolean get() = trustReset

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
     * The label as a screen may quote it (review M2, ov-560 R1): without
     * format characters (bidi controls that could reverse the line, zero-width
     * characters), opening and closing quotes but the apostrophe ’, or
     * double-quote lookalikes, any of which could close the quotes around it
     * and go on with a sentence of its own; runs of space made one; at most 40
     * characters. The relay, the web and AgentKit use the same rule, all
     * against `test/fixtures/device-labels.json`.
     */
    val shownLabel: String
        get() {
            val kept = label.codePoints().toArray().filter { !unshown(it) }
            val shown = StringBuilder()
            var i = 0
            while (i < kept.size) {
                if (kept[i] != APOSTROPHE) {
                    shown.appendCodePoint(kept[i]); i++; continue
                }
                var end = i
                while (end < kept.size && kept[end] == APOSTROPHE) end++
                // Once, and only between two letters.
                if (i > 0 && end < kept.size && Character.isLetter(kept[i - 1]) && Character.isLetter(kept[end])) {
                    shown.appendCodePoint(APOSTROPHE)
                }
                i = end
            }
            // Unicode spaces too (NBSP, em space): Java's \s is ASCII-only.
            val plain = shown.split(Regex("[\\s\\p{Z}]+")).filter { it.isNotEmpty() }.joinToString(" ")
            // Forty code points, not UTF-16 units, so a surrogate pair is never split.
            val cut = plain.substring(0, plain.offsetByCodePoints(0, minOf(40, plain.codePointCount(0, plain.length))))
            return cut.trim().ifEmpty { "a device" }
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
            "trusted" -> if (wasTrustedAgain) {
                "Confirmed when it signed in, since none of your devices was confirmed"
            } else {
                "Confirmed as the first device on your account"
            }
            "certified" -> "Confirmed by another of your devices"
            else -> if (anyConfirmed) {
                "Not confirmed yet. Scan its code from one of your confirmed devices."
            } else {
                "Not confirmed. None of your devices is confirmed, so whichever opens Far Cooler or " +
                    "signs in next becomes confirmed. Do that on the device you want to keep, then confirm " +
                    "the others from it."
            }
        }
        return if (isThisDevice) "This device. $words" else words
    }

    private fun blob(line: String): String? = line.split(" ").filter { it.isNotEmpty() }.getOrNull(1)

    companion object {
        private const val APOSTROPHE = 0x2019

        /**
         * The double quotes and lookalikes that are neither an opening nor a
         * closing quote (" ` „ ″ ‶ ＂ 〝 〞 〟 ❝ ❞ ˮ ״ 〃), and the letters that
         * draw as nothing (U+3164, U+115F, U+2800).
         */
        private val LOOKALIKES = setOf(
            0x22, 0x60, 0x201E, 0x2033, 0x2036, 0xFF02, 0x301D, 0x301E, 0x301F, 0x275D, 0x275E,
            0x02EE, 0x05F4, 0x3003, 0x3164, 0x115F, 0x2800,
        )
        private val DROPPED = setOf(
            Character.FORMAT, Character.CONTROL, Character.LINE_SEPARATOR, Character.PARAGRAPH_SEPARATOR,
            Character.INITIAL_QUOTE_PUNCTUATION, Character.FINAL_QUOTE_PUNCTUATION,
        )

        /** Whether [shownLabel] drops this code point. The apostrophe isn't: it's kept only between letters. */
        fun unshown(codePoint: Int): Boolean =
            codePoint != APOSTROPHE && (Character.getType(codePoint).toByte() in DROPPED || codePoint in LOOKALIKES)

        /**
         * Whether this phone should register again now, after a removal left
         * [rows] with no confirmed device and this phone pending (fix round
         * 1, Low 4): the person removing is on this phone, so it's the one
         * they're keeping, and R-75 makes it the confirmed one at once.
         */
        fun startsOverHere(rows: List<DeviceKeyRow>, ownKeyA: String?): Boolean =
            rows.none { it.isConfirmed } && rows.any { it.isHeldBy(ownKeyA) && it.state == "pending" }

        fun dayOf(ms: Long): String = java.text.DateFormat.getDateInstance(java.text.DateFormat.MEDIUM).format(java.util.Date(ms))

        /**
         * What the Remove dialog says, warning when [row] is the last confirmed
         * device among [rows] (review M2): removing it leaves nothing that can
         * confirm the others.
         */
        fun removalWarning(row: DeviceKeyRow, rows: List<DeviceKeyRow>): String =
            if (row.isConfirmed && rows.count { it.isConfirmed } == 1) {
                "This is the only device that can confirm your others. Removing it leaves them unconfirmed, " +
                    "and whichever opens Far Cooler or signs in next becomes confirmed in its place. " +
                    "Its key can’t be added back."
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
                trustedAt = ms("trustedAt"),
                trustReset = (row["trustReset"] as? JsonPrimitive)?.contentOrNull == "true",
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
