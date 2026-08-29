package com.farcooler.model

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * `stack_json`, transcribed key for key — and the words built off it.
 *
 * The same discipline `FleetDecodeTest` and `ChangesDecodeTest` follow, and for
 * a reason phase 8 got a fresh demonstration of: `BranchRef.remote` was declared
 * a `Boolean` against an `Option<String>` producer and the test that "covered"
 * it built its own payload with `"remote": true`, so it asserted the app's
 * mistake back at itself while the base picker failed to decode a single real
 * repository. A transcription is only worth having if it is copied from the
 * producer. The payload below is `crates/client/src/session.rs`'s `stack_json`,
 * every key it emits, with types as that function writes them.
 */
class StackTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun `a stack decodes every key stack_json emits`() {
        val payload = """
            {
              "cycleDetected": false,
              "prKnown": true,
              "repoUrl": "https://github.com/o/overnight",
              "links": [
                {
                  "branch": "feat/review-notes",
                  "parentBranch": "feat/review",
                  "parentGuessed": false,
                  "ahead": 3,
                  "behind": 1,
                  "pr": {
                    "number": 412,
                    "url": "https://github.com/o/overnight/pull/412",
                    "state": "open",
                    "checks": "failing",
                    "review": "changes_requested",
                    "headOid": "9f21c0d4e5f6",
                    "mergedAt": null,
                    "fetchedAt": 1785925800000,
                    "stale": true
                  }
                },
                {
                  "branch": "feat/review",
                  "parentBranch": "main",
                  "parentGuessed": true,
                  "ahead": 0,
                  "behind": 0,
                  "pr": null
                }
              ]
            }
        """.trimIndent()

        val reply = json.decodeFromString(StackReply.serializer(), payload)
        assertFalse(reply.cycleDetected)
        assertEquals(2, reply.links.size)
        // `gh` answered, and said what this repository is. The two facts that
        // decide whether an app may offer to create a pull request, and where
        // that offer would go.
        assertTrue(reply.prKnown)
        assertEquals("https://github.com/o/overnight", reply.repoUrl)

        val top = reply.links[0]
        assertEquals("feat/review-notes", top.branch)
        assertEquals("feat/review", top.parentBranch)
        assertFalse(top.parentGuessed)
        assertEquals(3, top.ahead)
        assertEquals(1, top.behind)

        val pr = requireNotNull(top.pr)
        assertEquals(412, pr.number)
        assertEquals("https://github.com/o/overnight/pull/412", pr.url)
        assertEquals("open", pr.state)
        assertEquals("failing", pr.checks)
        assertEquals("changes_requested", pr.review)
        assertEquals("9f21c0d4e5f6", pr.headOid)
        // Null and not zero: a pull request that has not landed has no date,
        // and 0 would date it to 1970.
        assertNull(pr.mergedAt)
        assertEquals(1_785_925_800_000L, pr.fetchedAt)
        assertTrue(pr.stale)

        val bottom = reply.links[1]
        assertTrue(bottom.parentGuessed)
        // Absent rather than an empty record: a branch with no pull request and
        // a pull request nobody could read are different answers.
        assertNull(bottom.pr)
    }

    /** A runner too old to send a key leaves it at its default, not at an error. */
    @Test
    fun `a stack with nothing in it still decodes`() {
        assertEquals(StackReply(), json.decodeFromString(StackReply.serializer(), "{}"))
        // The default that matters most. A runner that never sends `prKnown`
        // cannot have told us `gh` answered, and `false` is the reading that
        // keeps an app from offering to create a pull request that may already
        // exist.
        assertFalse(json.decodeFromString(StackReply.serializer(), "{}").prKnown)
        assertNull(json.decodeFromString(StackReply.serializer(), "{}").repoUrl)
        val one = json.decodeFromString(
            StackReply.serializer(), """{"links":[{"branch":"main"}]}"""
        )
        assertEquals("main", one.links.single().branch)
        assertFalse(one.links.single().parentGuessed)
        assertNull(one.links.single().pr)
    }

    /**
     * A loop is reported, not followed — and the links that came back are the
     * ones walked before it was noticed, so they are still drawn.
     */
    @Test
    fun `a cycle is a flag beside the links, not instead of them`() {
        val reply = json.decodeFromString(
            StackReply.serializer(),
            """{"cycleDetected":true,"links":[{"branch":"a","parentBranch":"b"}]}""",
        )
        assertTrue(reply.cycleDetected)
        assertEquals(1, reply.links.size)
    }

    @Test
    fun `drift says up to date rather than two zeroes`() {
        fun link(ahead: Int, behind: Int, parent: String = "main") =
            StackLink(branch = "x", parentBranch = parent, ahead = ahead, behind = behind)

        assertEquals("Up to date with main", link(0, 0).driftSentence())
        // A parent nobody recorded still gets a readable sentence rather than
        // "Up to date with ".
        assertEquals("Up to date with its parent", link(0, 0, parent = "").driftSentence())
        assertEquals("3 ahead", link(3, 0).driftSentence())
        assertEquals("2 behind", link(0, 2).driftSentence())
        assertEquals("3 ahead · 2 behind", link(3, 2).driftSentence())
    }

    /**
     * No wire word reaches the screen raw.
     *
     * `review_required` is the one that would show, and an underscore in the
     * middle of a sentence is a leaked wire value. Every unrecognized word falls
     * to something sayable rather than to itself, so a future `PrState` variant
     * this build predates cannot print an enum name at somebody.
     */
    @Test
    fun `pull request words are said, never spelled`() {
        assertEquals("Open", prStateWord("open"))
        assertEquals("Draft", prStateWord("draft"))
        assertEquals("Merged", prStateWord("merged"))
        assertEquals("Closed", prStateWord("closed"))
        assertEquals("Unknown", prStateWord("unknown"))
        assertEquals("Unknown", prStateWord("something_new"))

        assertEquals("Passing", prChecksWord("passing"))
        assertEquals("Failing", prChecksWord("failing"))
        assertEquals("Pending", prChecksWord("pending"))
        assertEquals("Unknown", prChecksWord("unknown"))

        assertEquals("Approved", prReviewWord("approved"))
        assertEquals("Changes requested", prReviewWord("changes_requested"))
        assertEquals("Review required", prReviewWord("review_required"))
        // Null so the row is left off entirely. "Review: Unknown" on every pull
        // request nobody has looked at — which is most of them, most of the
        // time — is a row spent to say nothing.
        assertNull(prReviewWord("unknown"))
        assertNull(prReviewWord(""))
    }
}
