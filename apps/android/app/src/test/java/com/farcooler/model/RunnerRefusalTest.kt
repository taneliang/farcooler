package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import java.io.File

/**
 * What a phone says when a runner refuses one request.
 *
 * Here rather than beside the sheet that draws it, because eight screens draw
 * one and eight copies of a decision is the drift this codebase keeps finding.
 *
 * Every way of getting this wrong is quiet. A word that drifts from the Rust
 * side matches nothing and the sentence silently reverts to the generic. A
 * caller that treats an unrecognized word as "no failure" shows nothing at all,
 * which is the bug that shipped in the agent-failure work. And a sentence that
 * quotes the word puts a proto identifier on a screen.
 *
 * Run by `./gradlew testInstrumentedUnitTest`, `.github/workflows/ci.yml:599`.
 * `AgentKitTests/RunnerRefusalTests.swift` is the Apple half; neither can see
 * the other, which is why both exist.
 */
class RunnerRefusalTest {

    /** The checkout, from this file, so the proto is findable without a fixture. */
    private fun repoRoot(): File {
        var dir = File("").absoluteFile
        while (!File(dir, "proto/farcooler.proto").isFile) {
            dir = dir.parentFile ?: error("no checkout above ${File("").absoluteFile}")
        }
        return dir
    }

    /**
     * Every word here is one a runner can really send.
     *
     * The other half of this wire is `farcooler_core::error::word`, which is
     * exhaustive over `ErrorCode` and cannot see Kotlin. So this reads the
     * proto itself — the one drift that would break all fourteen at once,
     * silently, by turning every sentence back into the generic one.
     */
    @Test
    fun everyWordNamesACodeTheProtocolDeclares() {
        val proto = File(repoRoot(), "proto/farcooler.proto").readText()
        for (refusal in RunnerRefusal.entries) {
            val declared = "ERROR_CODE_" + refusal.word.uppercase().replace('-', '_')
            assertTrue(
                "${refusal.word} is not a code the protocol declares ($declared)",
                proto.contains(declared),
            )
        }
    }

    /**
     * The word is read off the line the client core actually writes.
     *
     * **A gap this suite had, found by breaking it.** Blanking the field read in
     * `ClientCore.drain` left every one of these tests green — the table was
     * perfect and nothing ever reached it, which is exactly the failure this
     * whole change is about, one layer up. So the line that reads it moved into
     * [RunnerRefusal.wordInAnswerLine], where this can break it.
     *
     * The shapes below are `push_call`'s in `crates/client/src/ffi.rs`, which
     * the Rust test `a_refusal_reaches_the_line_with_the_runner_s_word_on_it`
     * pins from the writing side.
     */
    @Test
    fun theWordIsReadOffTheLineTheCoreWrites() {
        val json = Json { ignoreUnknownKeys = true }
        fun line(raw: String) = json.parseToJsonElement(raw).jsonObject

        val refused = line(
            """{"ticket":7,"ok":false,"disconnected":false,""" +
                """"error":"workspaces still exist under this resource",""" +
                """"code":"workspaces-exist"}"""
        )
        assertEquals("workspaces-exist", RunnerRefusal.wordInAnswerLine(refused))
        assertEquals(
            RunnerRefusal.WORKSPACES_EXIST.sentence,
            troubleFor(RunnerRefusal.wordInAnswerLine(refused), "raw", "Generic.").sentence,
        )

        // A dropped link carries no code at all — the key is absent, not null.
        val dropped = line(
            """{"ticket":8,"ok":false,"disconnected":true,"error":"not connected"}"""
        )
        assertNull(RunnerRefusal.wordInAnswerLine(dropped))

        // And an answer that worked carries neither.
        assertNull(RunnerRefusal.wordInAnswerLine(line("""{"ticket":9,"ok":true,"result":{}}""")))
    }

    /**
     * A word this build has never heard of still says something failed.
     *
     * **The rule, stated where it is decided.** A runner newer than this app
     * sends a code that is not in this enum yet; reading it as "nothing to
     * report" is how a screen ends up blank where it owes the reader a failure.
     * It falls back to the caller's own generic sentence with the runner's
     * words underneath, which is exactly what these screens showed before the
     * table existed — so an unknown code can only ever be as good as the old
     * behavior, never worse.
     */
    @Test
    fun aCodeFromTheFutureStillReadsAsAFailure() {
        val trouble = troubleFor(
            "from-the-future",
            "the runner said something this build cannot read",
            "Adding this repository didn’t finish.",
        )
        assertEquals("Adding this repository didn’t finish.", trouble.sentence)
        assertEquals("the runner said something this build cannot read", trouble.transcript)

        // The two words the client core sends for "no reason given" and "a
        // reason this build cannot read". Both are failures, neither a diagnosis.
        for (word in listOf("unspecified", "unrecognized")) {
            val t = troubleFor(word, "raw", "Generic.")
            assertEquals("$word must not claim a diagnosis", "Generic.", t.sentence)
            assertEquals("$word must keep the runner's words", "raw", t.transcript)
        }
    }

    /**
     * The codes that are real but have no sentence of their own, and why.
     *
     * Deliberately listed rather than left to the fallback by accident. If a
     * later change makes one of them reachable and worth a sentence, this test
     * is where somebody notices the decision was made.
     */
    @Test
    fun theCodesWithNoSentenceFallBackToTheCallersOwn() {
        val noSentence = listOf(
            // No `DomainError` variant produces it.
            "host-offline",
            // No production site anywhere in the tree.
            "dirty-worktree", "repository-locked", "output-gap", "attachment-limit",
            "dispatch-unknown", "diff-too-large", "diff-unsupported", "pr-state-unavailable",
            // Intercepted at the handshake as `SessionError::VersionMismatch`.
            "version-incompatible",
            // Raised on the local send; never put in a response.
            "client-too-slow",
            // No request path ever sets an idempotency key.
            "idempotency-mismatch",
            // The generic itself, and the one the client core answers structurally.
            "operation-failed", "confirmation-required",
        )
        for (word in noSentence) {
            assertNull("$word has a sentence but nothing can show it", RunnerRefusal.of(word))
            val t = troubleFor(word, "raw", "Generic.")
            assertEquals("Generic.", t.sentence)
            assertEquals("raw", t.transcript)
        }
    }

    /**
     * Nothing refused anything, so there is nothing to diagnose.
     *
     * A dropped link and an argument this app rejected before sending both
     * arrive with no word. They are still failures — the caller's sentence and
     * the words it has — just not ones a runner named.
     */
    @Test
    fun aFailureNoRunnerNamedKeepsTheCallersSentence() {
        for (word in listOf(null, "")) {
            val t = troubleFor(word, "not connected", "Generic.")
            assertEquals("Generic.", t.sentence)
            assertEquals("not connected", t.transcript)
        }
    }

    /**
     * A word we do know replaces the sentence and drops the transcript.
     *
     * The transcript goes because we have a diagnosis of our own. The core's
     * own text under one of these says strictly less than the sentence above it
     * — "workspaces still exist under this resource" under "Remove those first"
     * — so keeping it would be noise rather than diagnosis.
     */
    @Test
    fun aKnownWordSpeaksForItselfAndNeedsNoTranscript() {
        for (refusal in RunnerRefusal.entries) {
            val t = troubleFor(refusal.word, "the core’s own log line", "Generic.")
            assertEquals(refusal.sentence, t.sentence)
            assertTrue("${refusal.word} did not replace the generic", t.sentence != "Generic.")
            assertNull("${refusal.word} kept a transcript it does not need", t.transcript)
            assertNotNull(RunnerRefusal.of(refusal.word))
        }
    }

    /**
     * A step that failed keeps its own sentence and gains the reason.
     *
     * Quick Task's arms are the reason this exists: "Created the worktree, but
     * couldn't start Claude." says how much of the job got done, and losing
     * that to say why would be a worse screen, not a better one.
     */
    @Test
    fun aStepThatFailedKeepsItsOwnSentenceAndGainsTheReason() {
        val step = "Created the worktree, but couldn’t start Claude."
        val known = troubleAfter(RunnerRefusal.TMUX_UNAVAILABLE.word, "tmux is unavailable", step)
        assertTrue(known.sentence.startsWith("$step "))
        assertTrue(known.sentence.endsWith(RunnerRefusal.TMUX_UNAVAILABLE.sentence))
        assertNull(known.transcript)

        val unknown = troubleAfter("from-the-future", "tmux is unavailable", step)
        assertEquals(step, unknown.sentence)
        assertEquals("tmux is unavailable", unknown.transcript)
    }

    /** The word the runner sent must never be the words a person reads. */
    @Test
    fun noSentenceQuotesTheMachineWordBack() {
        for (refusal in RunnerRefusal.entries) {
            val sentence = refusal.sentence
            assertTrue("${refusal.word} is quoted at the reader", !sentence.contains(refusal.word))
            assertTrue(!sentence.contains("ERROR_CODE"))
            assertTrue(!sentence.contains(refusal.name))
            // The core's own text is written for a daemon log. None of it
            // belongs in prose.
            assertTrue(!sentence.lowercase().contains("resource"))
            assertTrue("a sentence must never carry a path", !sentence.contains("/"))
        }
    }

    /** Fourteen sentences, fourteen different things to do. */
    @Test
    fun eachRefusalSaysSomethingOfItsOwn() {
        val seen = mutableSetOf<String>()
        for (refusal in RunnerRefusal.entries) {
            assertTrue("${refusal.word} reuses a sentence", seen.add(refusal.sentence))
        }
        val words = RunnerRefusal.entries.map { it.word }
        assertEquals("two entries share a word", words.size, words.toSet().size)
    }

    /**
     * This app's voice: a real sentence, and a curly apostrophe in a
     * contraction, matching the 122 that were already here.
     */
    @Test
    fun everySentenceIsReadableInThisAppsVoice() {
        for (refusal in RunnerRefusal.entries) {
            val sentence = refusal.sentence
            assertTrue("${refusal.word} says too little to act on", sentence.length > 30)
            assertTrue("${refusal.word} is not a sentence", sentence.endsWith("."))
            assertTrue("${refusal.word} uses a straight apostrophe", !sentence.contains("'"))
            assertTrue("${refusal.word} does not start a sentence", sentence.first().isUpperCase())
        }
    }

    /**
     * Word for word with the Apple apps.
     *
     * The same runner refusing the same thing must not be described two ways
     * depending on which phone is in your hand. `RunnerSettingsCopyTest`,
     * `RendezvousCopyTest` and `AgentEmptyStateTest` pin the same property for
     * their own copy, and for the same reason: Kotlin cannot import the Swift,
     * so the only thing holding the two together is a test that reads it.
     */
    @Test
    fun everySentenceIsTheOneTheAppleAppsSay() {
        val swift = File(
            repoRoot(),
            "apps/shared/AgentKit/Sources/AgentKit/RunnerRefusal.swift",
        ).readText()
        // The Swift wraps its longer sentences across lines with `+`, so the
        // literals are joined back together before comparing.
        val joined = Regex("\"\\s*\\+\\s*\"").replace(swift, "")
        for (refusal in RunnerRefusal.entries) {
            assertTrue(
                "${refusal.word}: the Apple apps do not say \"${refusal.sentence}\"",
                joined.contains("\"${refusal.sentence}\""),
            )
            assertTrue(
                "${refusal.word} is not a case the Apple apps have",
                joined.contains("= \"${refusal.word}\""),
            )
        }
        // And no case there that is missing here, which a one-way check would
        // let through: fourteen sentences on one phone and thirteen on the
        // other is the same drift, pointing the other way.
        val theirWords = Regex("case \\w+ = \"([a-z-]+)\"").findAll(joined)
            .map { it.groupValues[1] }.toSet()
        assertEquals(RunnerRefusal.entries.map { it.word }.toSet(), theirWords)
    }
}
