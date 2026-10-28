package com.farcooler.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull

/**
 * The devices that may log in to one runner, who added each, and what revoking one takes with it (ov-855), as
 * `client.list` answers: AgentKit's `EnrolledDevices`, in this platform's sentence case.
 *
 * One row per device, not per line: a Mac holds two lines under one client id and they are removed together. A
 * foreign line has no client id and is never touched, so it has no row.
 */
data class EnrolledDevices(val devices: List<Device>) {
    /** [addedBy] is the client id of the device that added this one, or "" for one nobody is recorded as adding. */
    data class Device(val clientId: String, val label: String, val scope: String = "", val addedBy: String = "") {
        /** What a row calls it: the name the device gave, or its id when it gave none. */
        val title: String get() = label.ifEmpty { clientId }
    }

    /** "Added by Alexandra’s iPhone", or null for a device nobody added. */
    fun addedByLine(device: Device): String? {
        if (device.addedBy.isEmpty()) return null
        val adder = devices.firstOrNull { it.clientId == device.addedBy } ?: return ADDER_REMOVED
        return "Added by ${adder.title}"
    }

    /** What the chain of `added_by` from [device] reaches: the set `ClientRevoke.with_added` takes. Never [device]. */
    fun addedBy(device: Device): List<Device> {
        val reached = mutableSetOf(device.clientId)
        val frontier = ArrayDeque(listOf(device.clientId))
        val found = mutableListOf<Device>()
        while (frontier.isNotEmpty()) {
            val parent = frontier.removeLast()
            for (child in devices) {
                if (child.addedBy == parent && reached.add(child.clientId)) {
                    found += child
                    frontier += child.clientId
                }
            }
        }
        return found
    }

    fun revokeChoices(device: Device): RevokeChoices = RevokeChoices(device, addedBy(device))

    /** The two ways to revoke, or the one when the device added nothing. */
    data class RevokeChoices(val device: Device, val added: List<Device>) {
        val title: String get() = "Revoke ${device.title}?"

        /** "Revoke This and the 2 It Added", or null when it added none: a second choice that does the same is no choice. */
        val withAddedTitle: String? get() = if (added.isEmpty()) null else "Revoke this and the ${added.size} it added"

        val message: String
            get() = buildList {
                if (added.isNotEmpty()) {
                    add("${device.title} added ${if (added.size == 1) "1 device" else "${added.size} devices"}: ${added.joinToString(", ") { it.title }}.")
                }
                add("A shell the device already has open stays open.")
            }.joinToString(" ")
    }

    companion object {
        const val JUST_THIS_TITLE = "Revoke this"
        const val SECTION_TITLE = "Devices on this runner"
        const val EMPTY = "No other devices have access to this runner."
        const val ADDER_REMOVED = "Added by a device that was removed"

        /** A revoke the runner didn't take. No cause invented, no raw error. */
        const val NOT_REVOKED = "Couldn’t revoke that device. Check the runner is reachable, then try again."

        /** A `client.list` or `client.revoke` answer, or null when it isn't one. */
        fun of(reply: JsonObject): EnrolledDevices? {
            val lines = reply["clients"] as? JsonArray ?: return null
            fun JsonObject.text(key: String) = (this[key] as? JsonPrimitive)?.contentOrNull
            val seen = mutableSetOf<String>()
            val devices = lines.mapNotNull { it as? JsonObject }.mapNotNull { line ->
                val id = line.text("clientId") ?: ""
                if ((line["foreign"] as? JsonPrimitive)?.booleanOrNull == true || id.isEmpty() || !seen.add(id)) {
                    null
                } else {
                    Device(id, line.text("label") ?: "", line.text("scope") ?: "", line.text("addedBy") ?: "")
                }
            }
            return EnrolledDevices(devices)
        }
    }
}
