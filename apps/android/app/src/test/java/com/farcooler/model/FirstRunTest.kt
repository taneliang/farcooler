package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** First run (ov-205), the same table as AgentKit's `FirstRunTests`. */
class FirstRunTest {
    @Test
    fun `a command not found at the start is not installed`() {
        assertEquals(OrchestratorExit.NOT_INSTALLED, OrchestratorExit.classify(127, 3_000))
        assertEquals(OrchestratorExit.NOT_INSTALLED, OrchestratorExit.classify(127, 15_000))
    }

    @Test
    fun `anything else is an ordinary stop`() {
        assertNull(OrchestratorExit.classify(127, 15_500))
        assertNull(OrchestratorExit.classify(127, 60_000))
        assertNull(OrchestratorExit.classify(1, 3_000))
        assertNull(OrchestratorExit.classify(0, 3_000))
        assertNull(OrchestratorExit.classify(null, 3_000))
    }

    @Test
    fun `cursor is found as cursor-agent`() {
        val cursor = HarnessAvailability(listOf("cursor-agent"))
        assertTrue(cursor.isInstalled(AgentHarness.CURSOR))
        assertEquals(listOf(AgentHarness.CURSOR), cursor.installed)
        assertEquals(listOf(AgentHarness.CLAUDE, AgentHarness.CODEX), cursor.missing)
        // `cursor` is the editor's command, not the agent's.
        assertFalse(HarnessAvailability(listOf("cursor")).isInstalled(AgentHarness.CURSOR))
    }

    @Test
    fun `the programs are the ones the runner sends`() {
        // `wire::AGENT_PROGRAMS` in crates/daemon, in the same order.
        assertEquals(listOf("claude", "codex", "cursor-agent"), AgentHarness.entries.map { it.program })
        assertEquals(listOf("claude", "codex", "cursor"), AgentHarness.entries.map { it.wire })
        assertEquals(listOf("Claude Code", "Codex", "Cursor"), AgentHarness.entries.map { it.title })
    }

    @Test
    fun `a runner that did not say offers every harness`() {
        val unknown = HarnessAvailability(null)
        assertFalse(unknown.isKnown)
        assertEquals(AgentHarness.entries, unknown.installed)
        assertTrue(unknown.missing.isEmpty())
        val none = HarnessAvailability(emptyList())
        assertTrue(none.installed.isEmpty())
        assertEquals(AgentHarness.entries, none.missing)
    }

    @Test
    fun `not installed names the command, the runner, and the Cursor CLI`() {
        assertEquals("Claude Code isn’t installed", FirstRunCopy.notInstalledTitle(AgentHarness.CLAUDE))
        assertEquals("Cursor CLI isn’t installed", FirstRunCopy.notInstalledTitle(AgentHarness.CURSOR))
        assertEquals(
            "There’s no cursor-agent command on build-01. Install the Cursor CLI there, then try again.",
            FirstRunCopy.notInstalledBody(AgentHarness.CURSOR, "build-01"),
        )
    }

    @Test
    fun `the explainer promises nothing about a closed app`() {
        assertFalse(FirstRunCopy.NOTIFY_BODY.contains("closed"))
    }

    /** The voice check, the same rules as AgentKit's. */
    @Test
    fun `every string reads in the app's own voice`() {
        val stock = listOf("seamless", "effortless", "unlock", "supercharge", "elevate", "dive in", "get started", "let’s")
        assertTrue(FirstRunCopy.all.size > 30)
        for (string in FirstRunCopy.all) {
            assertFalse("an exclamation mark: $string", string.contains("!"))
            assertFalse("a straight apostrophe: $string", string.contains("'"))
            assertFalse("a spaced em dash: $string", string.contains(" — "))
            for (phrase in stock) assertFalse("\"$phrase\": $string", string.lowercase().contains(phrase))
        }
    }

    /** Sentence case: after a sentence's first word, a capital only in a proper name. */
    @Test
    fun `every string is in sentence case`() {
        val proper = setOf("Far", "Cooler", "Claude", "Code", "Codex", "Cursor", "CLI", "Mac", "Linux", "Main")
        for (string in FirstRunCopy.all) {
            // A Mac menu path is quoted in the Mac's own casing.
            val ours = string.replace("File > Add Repository", "")
            ours.split(". ", ": ").forEach { sentence ->
                sentence.split(" ").drop(1).map { it.trimEnd('.', ',') }
                    .filter { it.firstOrNull()?.isUpperCase() == true }
                    .forEach { word -> assertTrue("title case in \"$string\": $word", word in proper) }
            }
        }
    }
}
