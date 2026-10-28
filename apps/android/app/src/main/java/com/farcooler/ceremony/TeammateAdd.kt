package com.farcooler.ceremony

import com.farcooler.account.RelayAnswer
import com.farcooler.account.body
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

// Adding a teammate's device from this phone (ov-854), in the Mac's and the
// iPhone's words: `TeammateAdd.swift` in AgentKit holds the same sentences, and
// `crates/client/src/session/teammate.rs` is the one place that writes the key
// and binds it. This file is what the Android app says and sends.

/**
 * Who the team directory says a teammate is (ov-525): the account their code
 * named, and the email and team the DIRECTORY has for it. Nothing here came
 * from the code but the account it was looked up by.
 */
@Serializable
data class CeremonyTeammate(val account: String, val email: String, val team: String)

/**
 * A teammate's access on the runners they're added to: Control or Read, never a
 * shell (ruling R-65). The core refuses a teammate's code that asks for a shell
 * key, and this is the only choice left.
 */
enum class TeammateScope(val word: String, val title: String, val detail: String) {
    CONTROL("control", "Control", "They can run agents and commands on the runners you select."),
    READ("read", "Read", "They can follow the work on the runners you select, but can’t change it."),
}

/**
 * A teammate's code: who the directory says they are, and what the person
 * adding them chose.
 */
data class TeammateChoice(
    val who: CeremonyTeammate,
    /** The directory's answers the scan was checked against, kept to check the same code again at the tap. */
    val directory: String,
    /** This phone's email, written into the reply as whose runners these are. */
    val grantedBy: String,
    val handle: String = TeammateNames.handle(who.email),
    val name: String = TeammateNames.name(who.email),
    val scope: TeammateScope = TeammateScope.CONTROL,
) {
    /** Whether what was typed is something a runner will take. */
    val isComplete: Boolean get() = TeammateNames.isValidHandle(handle) && name.isNotBlank()
}

/**
 * What a teammate added by QR is called on a runner (ov-525, ruling R-65),
 * before anyone edits it: a handle from their email's local part, and a name to
 * read. Suggestions only. The runner decides what's a handle
 * (`valid_handle` in `crates/store/src/people.rs`) and makes it unique, so
 * [isValidHandle] is the same rule spelled here only to say so before the
 * person taps Add. The same rules as AgentKit's `TeammateNames`.
 */
object TeammateNames {
    /** The handles the runner keeps for itself (`RESERVED` in `people.rs`). */
    private val reserved = setOf("owner", "orchestrator", "rep", "runner", "manager", "user")

    /** 1 to 32 lowercase letters, digits or `-`, starting with a letter, and none the runner keeps. */
    fun isValidHandle(handle: String): Boolean =
        handle.length in 1..32 &&
            handle.first() in 'a'..'z' &&
            handle.all { it in 'a'..'z' || it in '0'..'9' || it == '-' } &&
            handle !in reserved

    /** `bob.smith@example.com` is `bob-smith`. */
    fun handle(email: String): String {
        val local = email.substringBefore('@')
        val base = StringBuilder()
        for (c in local.lowercase()) {
            if (c in 'a'..'z' || c in '0'..'9') {
                base.append(c)
            } else if (base.isNotEmpty() && !base.endsWith('-')) {
                base.append('-')
            }
        }
        // It must start with a letter: digits in front are dropped, not kept.
        var text = base.toString().dropWhile { it !in 'a'..'z' }.take(32).trimEnd('-')
        if (text.isEmpty()) text = "teammate"
        return if (text in reserved) "$text-2" else text
    }

    /** `bob.smith@example.com` reads as "Bob Smith", within the runner's 64 characters. */
    fun name(email: String): String {
        val local = email.substringBefore('@')
        val words = local.split(*charArrayOf('.', '-', '_', '+')).flatMap { part ->
            part.split(Regex("[0-9]+"))
        }.filter { it.isNotEmpty() }
        val name = if (words.isEmpty()) email else words.joinToString(" ") { word ->
            word.lowercase().replaceFirstChar { it.uppercase() }
        }
        return name.take(64)
    }
}

/**
 * The core's answer to a teammate's code it accepted
 * (`farcooler_client_ceremony_scan_teammate`): the offer re-encoded, and who the
 * directory says they are.
 */
@Serializable
data class TeammateAccepted(val offer: String, val teammate: CeremonyTeammate)

/**
 * What became of one runner's half of Add Teammate: `teammate.enroll`'s answer
 * (ov-854), which `session/teammate.rs` words and this decodes.
 */
sealed interface TeammateAddOutcome {
    /** Key A is on the runner and bound to the teammate; [tunnel] is the runner's token, or empty. */
    data class Written(val tunnel: String) : TeammateAddOutcome

    /** This phone isn't a `host_admin` device there; nothing was written. */
    data object NeedsAdmin : TeammateAddOutcome

    /** The runner's Far Cooler is too old to keep a member's account. */
    data object TooOld : TeammateAddOutcome

    /** Who's on the runner couldn't be read, so nothing was written. */
    data object CouldNotRead : TeammateAddOutcome

    /** The device is already another member's there. */
    data object SomeoneElses : TeammateAddOutcome

    /** The write failed, and the key was taken off again ([stillThere]: or couldn't be). */
    data class NotAdded(val stillThere: Boolean) : TeammateAddOutcome

    /** The key went on and couldn't be bound, so it was taken off again. */
    data class NotRecorded(val stillThere: Boolean) : TeammateAddOutcome

    /** The device was bound before, and its record couldn't be updated. */
    data object NotUpdated : TeammateAddOutcome

    /** The phone never reached the runner, or the runner refused the call. */
    data object NotReached : TeammateAddOutcome

    val isWritten: Boolean get() = this is Written

    /**
     * The reason a reply carries for a runner this left pending
     * (`RunnerEntry.pending_why`), or null for a bound one.
     */
    val pendingWhy: String?
        get() = when (this) {
            is Written -> null
            TooOld -> "too_old"
            NeedsAdmin -> "not_allowed"
            NotReached -> "not_connected"
            else -> "failed"
        }

    companion object {
        /** Decode `teammate.enroll`'s JSON. An answer that doesn't read is [NotReached], never a guess at success. */
        fun decode(answer: JsonObject?): TeammateAddOutcome {
            val word = answer?.get("outcome")?.jsonPrimitive?.takeIf { it.isString }?.contentOrNull
                ?: return NotReached
            val stillThere = answer["stillThere"]?.jsonPrimitive?.booleanOrNull ?: false
            return when (word) {
                "written" -> Written(answer["tunnel"]?.jsonPrimitive?.contentOrNull.orEmpty())
                "needs_admin" -> NeedsAdmin
                "too_old" -> TooOld
                "could_not_read" -> CouldNotRead
                "someone_elses" -> SomeoneElses
                "not_added" -> NotAdded(stillThere)
                "not_recorded" -> NotRecorded(stillThere)
                "not_updated" -> NotUpdated
                else -> NotReached
            }
        }

        fun decode(text: String): TeammateAddOutcome =
            decode(runCatching { Json.parseToJsonElement(text).jsonObject }.getOrNull())
    }
}

/**
 * The sentences for Add Teammate on a phone, the iPhone's word for word
 * (`TeammateAddCopy`). The Mac's are the same where it says the same thing.
 */
object TeammateAddCopy {
    /**
     * What to say about the runners that didn't take the teammate's device, one
     * sentence for each, or null when they all did. [device] is what this phone
     * is called in a sentence.
     */
    fun note(about: List<Pair<String, TeammateAddOutcome>>, device: String): String? =
        about.mapNotNull { (runner, outcome) -> sentence(outcome, runner, device) }
            .takeIf { it.isNotEmpty() }
            ?.joinToString("\n\n")

    /** One runner's sentence, or null when the device is there and bound. */
    fun sentence(outcome: TeammateAddOutcome, runner: String, device: String): String? = when (outcome) {
        is TeammateAddOutcome.Written -> null
        TeammateAddOutcome.NeedsAdmin ->
            "This $device can only control $runner, so it can’t add a teammate there. " +
                "Add your teammate from your Mac."
        TeammateAddOutcome.TooOld ->
            "$runner’s Far Cooler is too old to record whose device this is, so nothing was added there. " +
                "Update Far Cooler on it, then add your teammate again."
        TeammateAddOutcome.CouldNotRead ->
            "Far Cooler couldn’t read who’s on $runner, so it didn’t add your teammate’s device there."
        TeammateAddOutcome.SomeoneElses ->
            "This device is already someone else’s on $runner, so it wasn’t added for your teammate."
        is TeammateAddOutcome.NotAdded ->
            "Far Cooler couldn’t add your teammate’s device to $runner, and any access it had there before " +
                "is gone. " + if (outcome.stillThere) STILL_THERE else TAKEN_OFF
        is TeammateAddOutcome.NotRecorded ->
            "Far Cooler couldn’t record this device as your teammate’s on $runner. " +
                if (outcome.stillThere) STILL_THERE else TAKEN_OFF
        TeammateAddOutcome.NotUpdated ->
            "Far Cooler couldn’t update your teammate’s record on $runner, but their device still has " +
                "the access you chose. Add your teammate again to finish updating it."
        TeammateAddOutcome.NotReached ->
            "This $device couldn’t reach $runner, so your teammate’s device wasn’t added there."
    }

    const val TAKEN_OFF = "It took the device’s key off again, so it has no access there."
    const val STILL_THERE =
        "It also couldn’t make sure the device’s key isn’t on that runner. Check This device > Devices and runners for it." // casing ok: the screen's name
}

/**
 * What a teammate's code is refused with, in the Mac's words (ov-525) and the
 * iPhone's: the core's word picks the case; these are its sentences.
 */
object TeammateRefusalCopy {
    const val DIRECTORY_UNREACHABLE_TITLE = "Couldn’t check your teams"
    const val DIRECTORY_UNREACHABLE =
        "That device uses another account, and Far Cooler couldn’t reach your team directory to " +
            "check it’s a teammate’s, so nothing was added. Check your connection, then try again."

    const val NOT_A_TEAMMATE_TITLE = "This device’s account isn’t on any of your teams"
    fun notATeammate(email: String): String =
        "Invite its owner to one of your teams, or sign in to $email on that device, then show " +
            "its code again."

    const val TEAMMATE_SHELL_TITLE = "Teammates can’t have shell access"
    const val TEAMMATE_SHELL = "On that Mac, turn off shell access, then show its code again."

    const val UNREGISTERED_KEY_TITLE = "Their account hasn’t confirmed that device"
    const val UNREGISTERED_KEY =
        "Far Cooler adds a teammate’s device only once their account has confirmed it. " +
            "Ask them to sign in to Far Cooler on it, then scan its code from another of their " +
            "devices that’s already confirmed. If it’s their first device, signing in is enough. " +
            "Then scan its code here again."

    /** A phone has no way to confirm a held device in person, so it refuses one and sends the person to the Mac. */
    const val HELD_KEY_TITLE = "Their device is waiting out a hold"
    const val HELD_KEY_ON_A_PHONE =
        "Their account confirmed it after it had no confirmed device left, so your team can add it once " +
            "the hold ends. Add it from your Mac, where you can confirm it in person."
}

/**
 * The call a phone makes to a runner for one Add Teammate (ov-854), in one place
 * so the call and its test read the same words; the iPhone sends the same names
 * from AgentKit's `TeammateAddRequest`.
 *
 * **One call, not `client.enroll` and then a bind.** `teammate.enroll` is the
 * client core's: it writes the key `for_teammate` (so the runner holds it
 * nobody's until it is bound), binds it to the member and takes it off again if
 * either step fails. A phone that sent a plain `client.enroll` would leave the
 * teammate's key the owner's.
 */
object TeammateAddRequest {
    const val METHOD = "teammate.enroll"

    fun arguments(
        publicKey: String,
        label: String,
        clientId: String,
        nodeKey: String,
        teammate: TeammateChoice,
    ): Map<String, String> = mapOf(
        "publicKey" to publicKey,
        "label" to label,
        "clientId" to clientId,
        // Control or Read; the core refuses anything wider.
        "scope" to teammate.scope.word,
        "nodeKey" to nodeKey,
        "handle" to teammate.handle,
        "name" to teammate.name.trim(),
        // The directory's, never the code's.
        "account" to teammate.who.account,
        "email" to teammate.who.email,
    )
}

/**
 * The directory's own answers about who shares a team with the signed-in
 * person, for the ceremony's teammate check (ov-525): its answer to `me`, and
 * its answer to `teams/members` for each team that isn't personal, as
 * `{"me": …, "rosters": […]}`. AgentKit's `DirectoryClient.teamDirectory` asks
 * the same way.
 *
 * Passed on as the relay sent them, unjudged: which teams and roles count is
 * `crates/ceremony/src/teammate.rs`'s to decide. A roster the relay refused is
 * left out, and the core treats a team it needed and doesn't have as an answer
 * it can't use. Null when `me` itself didn't answer, which the core refuses as
 * unreachable.
 */
object TeamDirectory {
    const val ME = "/v1/directory/me"
    const val MEMBERS = "/v1/directory/teams/members"

    suspend fun fetch(post: suspend (path: String, body: JsonObject) -> RelayAnswer): String? {
        val me = post(ME, JsonObject(emptyMap())).body ?: return null
        val rosters = mutableListOf<JsonObject>()
        val teams = runCatching { me["teams"]?.jsonArray }.getOrNull().orEmpty()
        for (team in teams) {
            val row = team as? JsonObject ?: continue
            if (row["personal"]?.jsonPrimitive?.booleanOrNull != false) continue
            val id = row["id"]?.jsonPrimitive?.contentOrNull ?: continue
            val roster = post(MEMBERS, buildJsonObject { put("team", JsonPrimitive(id)) }).body ?: continue
            rosters += roster
        }
        return buildJsonObject {
            put("me", me)
            put("rosters", JsonArray(rosters))
        }.toString()
    }
}
