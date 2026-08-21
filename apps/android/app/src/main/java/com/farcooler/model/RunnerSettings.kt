package com.farcooler.model

import com.farcooler.data.Theme
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import kotlinx.serialization.json.putJsonObject

/**
 * What the Test button in the adapter editor found.
 *
 * Mirrors AgentKit's `AdapterTestOutcome`, in
 * `apps/shared/AgentKit/Sources/AgentKit/AdapterTest.swift`, which the Mac and
 * the phone both read from. The cases name a SITUATION and never carry a
 * sentence: a call site chooses which way a test failed and
 * [sentence] chooses the words, so what three platforms say cannot come apart
 * one edit at a time.
 */
sealed interface AdapterTestOutcome {
    /**
     * It started, it answered, and this is what it called itself.
     *
     * A name, not a sentence — the agent's own `agentInfo`, or the version the
     * native backend checked. So it is joined to the sentence rather than boxed
     * as output: a runner's DIAGNOSIS is what must never arrive in this app's
     * voice, and a runner's name for itself is the news.
     */
    data class Worked(val reported: String) : AdapterTestOutcome

    /** It didn't. */
    data class Failed(val reason: Reason) : AdapterTestOutcome

    /**
     * Which way a test failed, and what the other side said about it.
     *
     * AgentKit has a third, `formUnusable`, and this deliberately does not.
     * That one is the Mac's alone: it pipes the form to the CLI as JSON and can
     * fail to encode it, where [com.farcooler.net.Connection.testAdapter] hands
     * a `JsonObject` to the bridge with no encoding step to fail. A case
     * nothing here can construct would be a sentence nobody here can read.
     */
    sealed interface Reason {
        /**
         * The request went out and nothing usable came back.
         *
         * [output] is whatever the transport printed. Always null here — a
         * bridge call that throws throws a state rather than a message — and
         * carried anyway, because the field is what makes the Mac's arm, which
         * does have the CLI's line, the same case rather than a fourth one.
         */
        data class NoAnswer(val output: String?) : Reason

        /**
         * The handshake ran on the runner and did not complete, in the runner's
         * own words.
         *
         * Those words are the only clue about which field is wrong, which is
         * why `adapter.test` in `crates/daemon/src/rpc.rs` passes the adapter's
         * message through rather than replacing it with "the test failed". They
         * are also lowercase fragments about a process — ``could not find `npx`
         * on this runner``, `the adapter started and then went silent` — so
         * they are shown as output and never as a sentence.
         */
        data class Refused(val said: String) : Reason
    }
}

/**
 * Whether to draw this as good news.
 *
 * An extension rather than a member for the reason [transcript] gives: these
 * three are the copy, and the copy is written next to its Apple twin.
 */
val AdapterTestOutcome.succeeded: Boolean get() = this is AdapterTestOutcome.Worked

/**
 * This app's own account of what happened.
 *
 * Matched word for word to AgentKit's `AdapterTestOutcome.sentence`. The
 * success line used to read "Starts and speaks ACP", with the reported name
 * after it, on all three platforms — and that is the defect three copies were
 * hiding. Only the Mac's `AdapterInfo` carries a `backend`, so only the Mac can
 * ask for the agent's native protocol, and when it does no ACP is spoken at
 * all: one sentence, true here and on the phone and false on the Mac. What Test
 * proves either way is that the program starts and answers, so that is what
 * this says.
 */
val AdapterTestOutcome.sentence: String
    get() = when (this) {
        is AdapterTestOutcome.Worked -> "Starts and answers — $reported"
        is AdapterTestOutcome.Failed -> when (reason) {
            is AdapterTestOutcome.Reason.NoAnswer -> "That runner couldn’t be reached."
            // The negation of the success line on purpose: "start and answer"
            // is the pair being proven, and a test fails when the pair does not
            // hold — whether the program was never found, never started,
            // started and went quiet, or answered with a refusal. No cause is
            // named because this side cannot know which of those it was, and a
            // guess sends somebody to change a setting that was never the
            // problem. The one account anybody has of it is [transcript].
            is AdapterTestOutcome.Reason.Refused -> "This adapter didn’t start and answer."
        }
    }

/**
 * The other side's own words, to be shown as output.
 *
 * Null wherever there is nothing to show, so the editor can ask before it
 * reserves the space — an empty box under a sentence reads as output that
 * failed to arrive. Empty counts as nothing: the daemon sends `failure` as `""`
 * on the success path and a client that lost the field decodes to the same
 * thing, and neither is a transcript.
 *
 * AgentKit spells this `detail`; here it is `transcript`, which is what
 * [Trouble] already calls the other side's own words. `7661b67` found the trap
 * that settles it — Kotlin resolves a member before an extension, so an
 * extension sharing a name with a data class property is silently bypassed by
 * any call site holding the concrete type.
 */
val AdapterTestOutcome.transcript: String?
    get() = when (this) {
        is AdapterTestOutcome.Worked -> null
        is AdapterTestOutcome.Failed -> when (val why = reason) {
            is AdapterTestOutcome.Reason.NoAnswer -> why.output?.takeIf { it.isNotEmpty() }
            is AdapterTestOutcome.Reason.Refused -> why.said.takeIf { it.isNotEmpty() }
        }
    }

/**
 * One `[adapters.<name>]` table, plus where it came from.
 *
 * Mirrors the Mac's and iOS's own `AdapterInfo`. Carried whole rather than as a
 * diff against the built-in, because the editor has to be able to show what an
 * override actually says — including a deliberately emptied array, which a diff
 * could not tell apart from "unset".
 */
@Serializable
data class AdapterInfo(
    val preset: String,
    val program: String = "",
    val args: List<String> = emptyList(),
    val env: Map<String, String> = emptyMap(),
    /**
     * Detection, as opposed to launch. A wrong value here does not fail loudly:
     * it stops the agent being recognized, which surfaces later as "chat mode
     * broke" from somewhere else. `adapter.test` cannot check these.
     */
    val commands: List<String> = emptyList(),
    val identity: List<String> = emptyList(),
    val blocked: List<String> = emptyList(),
    val working: List<String> = emptyList(),
    val origin: String = "user",
) {
    /**
     * Whether Far Cooler can host this agent as a chat at all.
     *
     * An adapter with no program is a recognized agent that stays a terminal,
     * which is a real supported state rather than a gap.
     */
    val chatCapable: Boolean get() = program.isNotBlank()

    /** How this reads under its name in a list. */
    fun subtitle(): String {
        val where = when (origin) {
            "builtIn" -> "Shipped"
            "override" -> "Overriding a shipped agent"
            "user" -> "Yours"
            else -> "Unknown"
        }
        if (!chatCapable) return "$where — terminal only"
        return "$where — ${(listOf(program) + args).joinToString(" ")}"
    }

    /**
     * The bridge's arguments for `adapter.upsert` and `adapter.test`.
     *
     * Built by hand rather than serialized from this class, because `origin` is
     * the daemon's to decide on the way back out — a client claiming one would
     * be claiming something only the daemon can know.
     */
    fun toJson(): JsonObject = buildJsonObject {
        put("preset", preset)
        put("program", program)
        putJsonArray("args") { args.forEach { add(JsonPrimitive(it)) } }
        putJsonObject("env") { env.forEach { (key, value) -> put(key, value) } }
        putJsonArray("commands") { commands.forEach { add(JsonPrimitive(it)) } }
        putJsonArray("identity") { identity.forEach { add(JsonPrimitive(it)) } }
        putJsonArray("blocked") { blocked.forEach { add(JsonPrimitive(it)) } }
        putJsonArray("working") { working.forEach { add(JsonPrimitive(it)) } }
    }
}

/** The bridge's arguments for `theme.upsert`. */
fun Theme.toJson(): JsonObject = buildJsonObject {
    put("name", name)
    put("dark", dark)
    put("background", background)
    put("foreground", foreground)
    put("cursor", cursor)
    putJsonArray("ansi") { ansi.forEach { add(JsonPrimitive(it)) } }
}

/**
 * One entry per line, blank lines dropped.
 *
 * Deliberately not trimmed further: a detection string can legitimately begin or
 * end with a space — several built-ins do, because that is what the agent
 * actually draws — and trimming would break exactly the ones that matter.
 */
fun linesToList(text: String): List<String> =
    text.split("\n").filter { it.isNotBlank() }

/**
 * `KEY=value` per line, splitting on the FIRST `=` only.
 *
 * A value can contain them, and a token or a URL routinely does.
 */
fun linesToEnv(text: String): Map<String, String> =
    text.split("\n").mapNotNull { line ->
        val at = line.indexOf('=')
        if (at <= 0) return@mapNotNull null
        val key = line.substring(0, at).trim()
        if (key.isEmpty()) null else key to line.substring(at + 1)
    }.toMap()

fun envToLines(env: Map<String, String>): String =
    env.toSortedMap().entries.joinToString("\n") { "${it.key}=${it.value}" }
