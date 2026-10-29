package com.farcooler.model

import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull

/**
 * What a person may do on a runner (ov-856): Owner, Admin, Member or Viewer. The words are AgentKit's
 * `PersonAccess`, so the Mac, the iPhone and this app say the same thing about the same runner.
 */
enum class PersonAccess(val wire: String, val title: String, val summary: String) {
    OWNER("owner", "Owner", "Everything, including who has access."),
    ADMIN("admin", "Admin", "Can add and remove people and their devices."),
    MEMBER("member", "Member", "Can work with agents and terminals."),
    VIEWER("viewer", "Viewer", "Can see the board and progress, but not terminals."),
    ;

    companion object {
        fun of(wire: String?): PersonAccess? = entries.firstOrNull { it.wire == wire }

        /** What a menu offers [chooser]: nobody is made an Owner, and only the Owner makes an Admin. */
        fun choices(chooser: PersonAccess): List<PersonAccess> = when (chooser) {
            OWNER -> listOf(ADMIN, MEMBER, VIEWER)
            ADMIN -> listOf(MEMBER, VIEWER)
            MEMBER, VIEWER -> emptyList()
        }
    }
}

/** Where a device's step-up key stands, as the runner tells it. */
enum class StepUpKeyStanding(val wire: String) {
    NONE(""),
    PROPOSED("proposed"),
    TRUSTED("trusted"),
    EXPIRED("expired"),
    ;

    companion object {
        fun of(wire: String?): StepUpKeyStanding = entries.firstOrNull { it.wire == (wire ?: "") } ?: NONE
    }
}

/**
 * The people on a runner and their devices, as the People screen draws them (ov-856, §8 of the design): each person
 * with their Access, under each their devices, then Check These Devices and Waiting for Approval when either has
 * any. Built from `member.list` (who, their access, each device's place and key) and `client.list` (each line: its
 * name, who added it, whether it is a shell). AgentKit's `PeopleAccess`, in Kotlin.
 */
class PeopleAccess private constructor(
    val people: List<Person>,
    val toCheck: List<Device>,
    val waiting: List<Device>,
    /** This device's own row, when the runner lists it. */
    val thisDevice: Device?,
    /** False from a runner before person access, which has no Access to show. */
    val recorded: Boolean,
    val yourAccess: PersonAccess,
) {
    data class Device(
        val id: String,
        val name: String,
        /** "Added by iPhone", or null for a device the runner added itself. */
        val addedBy: String?,
        val viewerOnThisDevice: Boolean,
        val hasShell: Boolean,
        val needsCheck: Boolean,
        val stepUp: StepUpKeyStanding,
        /** The six digits the device shows, while its key waits. */
        val code: String,
        /** How many devices this one added, and those they added in turn. */
        val added: Int,
        /** The person's handle, "" for the owner. */
        val person: String,
        val personName: String,
    ) {
        /** How many devices are in this device's removal: itself and what it added. */
        val removal: Int get() = 1 + added
    }

    data class Person(val handle: String, val name: String, val access: PersonAccess, val isYou: Boolean, val devices: List<Device>) {
        val key: String get() = handle.ifEmpty { "(owner)" }
    }

    /** Whether this device may approve [other]'s key: another device of the same person, with a trusted key. */
    fun mayApprove(other: Device, from: Device?): Boolean =
        from != null && from.id != other.id && from.stepUp == StepUpKeyStanding.TRUSTED &&
            from.person == other.person && other.stepUp == StepUpKeyStanding.PROPOSED

    companion object {
        const val VIEWER_BADGE = "Viewer on This Device"
        const val SHELL_BADGE = "Has a Shell"
        const val CHECK_HEADER = "Check These Devices"
        const val CHECK_FOOTER = "A teammate’s device added these. Remove any you don’t recognize."
        const val WAITING_HEADER = "Waiting for Approval"
        const val WAITING_FOOTER =
            "These devices asked to approve changes to who has access. Allow one only from another device of the same person."
        const val LIMIT_TO_VIEWER = "Limit to Viewer"
        const val REMOVE_DEVICE = "Remove Device…"

        /** The code as it is read aloud and compared: "418 207". */
        fun spoken(code: String): String = if (code.length == 6) "${code.take(3)} ${code.takeLast(3)}" else code

        /** The approval sheet's sentence: only allow it holding the device. */
        fun approvalPrompt(device: Device): String =
            "Only allow this if you’re holding ${device.name} right now. Check that it shows ${spoken(device.code)}."

        /** The warning on a device with a shell on [runner]. */
        fun shellWarning(runner: String): String = "This device has a shell on $runner, so it can do everything."

        /**
         * A key as Far Cooler names it in `authorized_keys`: `farcooler-iPhone-t7xq9vd8` is an iPhone. The prefix and the
         * fingerprint's tail are the file's, not a name.
         */
        fun tidy(label: String): String {
            var name = label
            for (prefix in listOf("farcooler-shell-", "farcooler-")) {
                if (name.startsWith(prefix)) {
                    name = name.removePrefix(prefix)
                    break
                }
            }
            val dash = name.lastIndexOf('-')
            if (dash >= 0 && name.length - dash == 9 && name.substring(dash + 1).all { it.isLetterOrDigit() }) {
                name = name.substring(0, dash)
            }
            return name.ifEmpty { label }
        }

        private fun JsonObject.text(key: String) = (this[key] as? JsonPrimitive)?.contentOrNull
        private fun JsonObject.flag(key: String) = (this[key] as? JsonPrimitive)?.booleanOrNull ?: false

        /** For each device, the devices it added and the ones they added in turn. */
        internal fun addedCounts(parent: Map<String, String>): Map<String, Int> {
            val counts = mutableMapOf<String, Int>()
            for (child in parent.keys) {
                val seen = mutableSetOf(child)
                var up = parent[child]
                while (up != null && up !in seen) {
                    counts[up] = (counts[up] ?: 0) + 1
                    seen += up
                    up = parent[up]
                }
            }
            return counts
        }

        /**
         * `member.list` and `client.list` as the client core answers them, with [thisDevice] the client id of the
         * device asking. Null when [members] isn't a member list.
         */
        fun of(members: JsonObject, clients: JsonObject? = null, thisDevice: String = "", ownerName: String = "Owner"): PeopleAccess? {
            val list = members["members"] as? JsonArray ?: return null
            val you = members.text("you") ?: ""
            val rows = (members["device_access"] as? JsonArray)?.mapNotNull { it as? JsonObject }.orEmpty()
            val rowOf = rows.associateBy { it.text("client_id") ?: "" }
            val lines = (clients?.get("clients") as? JsonArray)?.mapNotNull { it as? JsonObject }.orEmpty()
                .filter { !it.flag("foreign") && !it.text("clientId").isNullOrEmpty() }
            val byDevice = lines.groupBy { it.text("clientId")!! }
            val parent = lines.mapNotNull { l ->
                val by = l.text("addedBy").orEmpty()
                if (by.isEmpty()) null else l.text("clientId")!! to by
            }.toMap()
            val added = addedCounts(parent)
            fun nameOf(id: String) = byDevice[id]?.firstOrNull()?.text("label")?.let(::tidy) ?: id

            fun device(id: String, person: String, personName: String, row: JsonObject?): Device {
                val mine = byDevice[id].orEmpty()
                val by = mine.firstNotNullOfOrNull { it.text("addedBy")?.takeIf(String::isNotEmpty) }
                return Device(
                    id = id,
                    name = nameOf(id),
                    addedBy = by?.let { "Added by ${if (byDevice[it] == null) "another device" else nameOf(it)}" },
                    viewerOnThisDevice = row?.text("cap") == "viewer",
                    hasShell = mine.any { it.flag("shellAccess") },
                    needsCheck = row?.flag("check") ?: false,
                    stepUp = StepUpKeyStanding.of(row?.text("step_up")),
                    code = row?.text("step_up_code") ?: "",
                    added = added[id] ?: 0,
                    person = person,
                    personName = personName,
                )
            }

            val youAreOwner = you.isEmpty()
            val ownerDevices = rows.filter { it.text("person").isNullOrEmpty() }.map {
                device(it.text("client_id") ?: "", "", ownerName, it)
            }
            val people = mutableListOf<Person>()
            if (rows.isNotEmpty() || youAreOwner) {
                people += Person("", if (youAreOwner) ownerName else "Owner", PersonAccess.OWNER, youAreOwner, ownerDevices)
            }
            for (m in list.mapNotNull { it as? JsonObject }) {
                val handle = m.text("handle") ?: ""
                val name = m.text("name") ?: ""
                val devices = (m["devices"] as? JsonArray).orEmpty().mapNotNull { (it as? JsonPrimitive)?.contentOrNull }
                    .map { id -> device(id, handle, name, rowOf[id]) }
                people += Person(handle, name, PersonAccess.of(m.text("access")) ?: PersonAccess.MEMBER, you == handle, devices)
            }
            val every = people.flatMap { it.devices }
            return PeopleAccess(
                people = people,
                toCheck = every.filter { it.needsCheck },
                waiting = every.filter { it.stepUp == StepUpKeyStanding.PROPOSED },
                thisDevice = if (thisDevice.isEmpty()) null else every.firstOrNull { it.id == thisDevice },
                recorded = rows.isNotEmpty(),
                yourAccess = if (youAreOwner) PersonAccess.OWNER else people.firstOrNull { it.handle == you }?.access ?: PersonAccess.MEMBER,
            )
        }
    }
}
