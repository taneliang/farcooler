package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Decode real bytes the daemon emitted, captured from a live session that
 * dispatched three subagents.
 *
 * `AgentStream` drops what fails to decode, so a field this side spells
 * differently from the Rust side is not a crash and not a warning — it is a
 * chat that silently renders less than happened. Synthetic JSON in the other
 * tests cannot catch that, because the same author wrote both sides of it.
 *
 * The fixture is the same file the Swift suite reads, copied into this module's
 * test resources: one capture, two clients, one expectation.
 */
class LiveDecodeTest {
    @Test
    fun everyEventTheDaemonActuallySentDecodes() {
        val raw = javaClass.classLoader!!
            .getResourceAsStream("live_events.jsonl")!!
            .bufferedReader()
            .readText()
        val lines = raw.lineSequence().filter { it.isNotBlank() }.toList()
        assertTrue("no captured events", lines.isNotEmpty())

        var gaps = 0
        var parented = 0
        var dispatches = 0
        var summaries = 0
        val unparsed = mutableListOf<String>()

        for (line in lines) {
            when (val event = AgentEvent.decode(line)) {
                is AgentEvent.Gap -> {
                    gaps += 1
                    unparsed.add(line.take(120))
                }

                is AgentEvent.Message -> if (event.parent != null) parented += 1

                is AgentEvent.ToolCall -> {
                    if (event.parent != null) parented += 1
                    if (event.subagent) dispatches += 1
                }

                is AgentEvent.ToolUpdate -> {
                    if (event.parent != null) parented += 1
                    if (event.subagent != null) summaries += 1
                }

                else -> Unit
            }
        }

        assertEquals(
            "the daemon sent events this client cannot read: $unparsed",
            0,
            gaps,
        )
        println("decoded ${lines.size}: parented=$parented dispatches=$dispatches summaries=$summaries")
    }
}
