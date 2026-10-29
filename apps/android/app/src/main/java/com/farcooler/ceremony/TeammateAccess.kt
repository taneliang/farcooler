package com.farcooler.ceremony

import com.farcooler.model.PeopleAccess
import com.farcooler.model.PersonAccess
import com.farcooler.model.StepUpKeyStanding
import com.farcooler.model.StepUpReason
import com.farcooler.model.StepUpWords
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.put

// Add Teammate with a person and an access (ov-856, §8 of the design): who this device is on a runner, whether the
// runner already has the teammate, what to offer, and the one request a step-up signs. AgentKit's
// `TeammateAccess.swift`, in Kotlin: the same rules and words.

/** A person a runner already has under the scanned account. */
data class ExistingPerson(val handle: String, val name: String, val access: PersonAccess)

/**
 * What Add Teammate needs to know about one runner, read from `member.list`: what the person adding is there, and who
 * is there already.
 */
data class TeammateRunnerFacts(
    /** What this device's person is on the runner; null when the runner can't say, which reads as unable to add. */
    val caller: PersonAccess?,
    /** The people this device could be added to: the one under the scanned account, and any added by hand with none. */
    val candidates: List<ExistingPerson> = emptyList(),
    /** The handle of the one under the scanned account, if any. */
    val matching: String? = null,
    /** Every handle the runner has, so a new person's is never one of them. */
    val taken: Set<String> = emptySet(),
    /** This device holds a trusted step-up key there, so adding asks once for that exact request. */
    val signs: Boolean = false,
) {
    companion object {
        /** Read from a `member.list` answer for the teammate's [account]; null when it isn't one. */
        fun of(members: JsonObject, account: String, thisDevice: String = ""): TeammateRunnerFacts? {
            val access = PeopleAccess.of(members, null, thisDevice) ?: return null
            val rows = (members["members"] as? JsonArray).orEmpty().mapNotNull { it as? JsonObject }
            fun text(row: JsonObject, key: String) = (row[key] as? JsonPrimitive)?.contentOrNull.orEmpty()
            val fits = rows.filter { text(it, "account") == account || text(it, "account").isEmpty() }
            return TeammateRunnerFacts(
                caller = if (access.recorded) access.yourAccess else null,
                candidates = fits.map { row ->
                    val handle = text(row, "handle")
                    ExistingPerson(handle, text(row, "name"), access.people.firstOrNull { it.handle == handle }?.access ?: PersonAccess.MEMBER)
                },
                matching = if (account.isEmpty()) null else rows.firstOrNull { text(it, "account") == account }?.let { text(it, "handle") },
                taken = rows.map { text(it, "handle") }.toSet(),
                signs = access.thisDevice?.stepUp == StepUpKeyStanding.TRUSTED,
            )
        }
    }
}

/** Add Teammate's rules, once for the phones. */
object TeammateAccessRules {
    /** What the Access menu offers when adding to runners where the caller is [callers]: Admin only to the Owner. */
    fun choices(callers: List<PersonAccess?>): List<PersonAccess> =
        if (callers.all { it == PersonAccess.OWNER }) PersonAccess.choices(PersonAccess.OWNER) else PersonAccess.choices(PersonAccess.ADMIN)

    /** Why this runner can't be ticked, or null when it can. Only the Owner and an Admin add people. */
    fun blocked(facts: TeammateRunnerFacts?, runner: String): String? = when (facts?.caller) {
        PersonAccess.OWNER, PersonAccess.ADMIN -> null
        PersonAccess.MEMBER, PersonAccess.VIEWER -> "Only an Owner or an Admin can add people to $runner." // casing ok: Owner, Admin and Viewer are access names
        null -> "Update Far Cooler on $runner to add people from here."
    }

    /** The person and access a runner is sent: the existing person's when picked, else a new one nobody has. */
    fun bind(choice: TeammateChoice, facts: TeammateRunnerFacts?): Map<String, String> {
        val existing = choice.person?.let { handle -> facts?.candidates?.firstOrNull { it.handle == handle } }
        if (existing != null) {
            return mapOf(
                "handle" to existing.handle, "name" to existing.name, "account" to choice.who.account,
                "email" to choice.who.email, "access" to existing.access.wire,
            )
        }
        return mapOf(
            "handle" to unique(choice.handle, facts?.taken.orEmpty()), "name" to choice.name.trim(),
            "account" to choice.who.account, "email" to choice.who.email, "access" to choice.access.wire,
        )
    }

    /** [chosen], or with `-2`, `-3` and so on until no member here has it, cut so the suffix fits in 32. */
    fun unique(chosen: String, taken: Set<String>): String {
        val reserved = setOf("owner", "orchestrator", "rep", "runner", "manager", "user")
        if (chosen !in taken && chosen !in reserved) return chosen
        var n = 2
        while (true) {
            val suffix = "-$n"
            val candidate = chosen.take(32 - suffix.length).trimEnd('-') + suffix
            if (candidate !in taken) return candidate
            n++
        }
    }

    /** The signed `client.enroll` for one runner: the line at the access's scope, `for_teammate`, with the person and access. */
    fun arguments(
        publicKey: String,
        label: String,
        clientId: String,
        nodeKey: String,
        teammate: TeammateChoice,
        facts: TeammateRunnerFacts?,
    ): JsonObject {
        val bound = bind(teammate, facts)
        return buildJsonObject {
            put("publicKey", publicKey)
            put("label", label)
            put("clientId", clientId)
            put("nodeKey", nodeKey)
            put("scope", if (bound["access"] == PersonAccess.VIEWER.wire) "read" else "control")
            put("forTeammate", true)
            put("teammate", buildJsonObject { bound.forEach { (k, v) -> put(k, v) } })
        }
    }

    /** What the prompt says. */
    fun reason(device: String, runner: String, access: String): String =
        StepUpReason.add(device, runner, PersonAccess.of(access) ?: PersonAccess.MEMBER)

    /** What a refusal of Add Teammate says, in words that name no runner error. */
    fun refusal(word: String, runner: String, device: String): String = when (word) {
        "not_admin", "not_owners_device" -> "Only an Owner or an Admin can add people to $runner." // casing ok: Owner, Admin and Viewer are access names
        else -> StepUpWords.sentenceForRefusal(word, runner, device)
    }

    /** "Dan is already on studio as a Member, and keeps that access." */
    fun keeps(person: ExistingPerson, runner: String): String {
        val article = if (person.access.title.first() in "AEIOU") "an" else "a"
        return "${person.name} is already on $runner as $article ${person.access.title}, and keeps that access."
    }
}

/**
 * What an Add Teammate writes (ov-854): the key `for_teammate` on each runner,
 * bound to the teammate, answering what became of each. A runner missing from
 * the answer was never asked.
 */
fun interface TeammateEnroller {
    suspend fun enroll(
        publicKey: String,
        label: String,
        clientId: String,
        nodeKey: String,
        teammate: TeammateChoice,
        runners: List<CeremonyRunner>,
        facts: Map<String, TeammateRunnerFacts>,
    ): Map<String, TeammateAddOutcome>
}
