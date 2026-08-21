package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The Swift suite in `apps/shared/AgentKit/Tests/AgentKitTests/
 * AgentEventTests.swift`, translated case for case — same JSON, same
 * expectations, so the two decoders cannot drift.
 */
class AgentEventTest {
    @Test
    fun anAgentMessageDecodes() {
        val event = AgentEvent.decode("""{"Message":{"role":"Agent","text":"hello"}}""")
        val message = event as AgentEvent.Message
        assertEquals(Role.AGENT, message.role)
        assertEquals("hello", message.text)
    }

    @Test
    fun aSubagentsMessageDecodesWithItsParent() {
        // Without the parent a client cannot tell a subagent's words from the
        // dispatching agent's, which is exactly how they came to be rendered as
        // the same speaker.
        val event = AgentEvent.decode(
            """{"Message":{"role":"Agent","text":"I'll read the file.","parent":"toolu_01Wnr"}}"""
        )
        val message = event as AgentEvent.Message
        assertEquals("I'll read the file.", message.text)
        assertEquals("toolu_01Wnr", message.parent)
    }

    @Test
    fun aMessageWithoutAParentStillDecodes() {
        // Every event already in SQLite was written before this field existed.
        // If absence threw, every stored transcript would fail to render.
        val event = AgentEvent.decode("""{"Message":{"role":"Agent","text":"hello"}}""")
        assertNull((event as AgentEvent.Message).parent)
    }

    @Test
    fun aNullParentIsTheSameAsAnAbsentOne() {
        // serde writes `"parent":null` rather than omitting the field in some
        // builds. A decoder that read the JSON null as the string "null" would
        // file every top-level message under a block that does not exist.
        val event = AgentEvent.decode("""{"Message":{"role":"Agent","text":"hi","parent":null}}""")
        assertNull((event as AgentEvent.Message).parent)
    }

    @Test
    fun aDispatchDecodesAsOne() {
        // The Task row and an ordinary tool row are the same event but for this
        // flag, and only the dispatch owns a block.
        val event = AgentEvent.decode(
            """{"ToolCall":{"id":"t1","title":"Task","kind":"think","status":"Pending","locations":[],"subagent":true}}"""
        )
        assertTrue((event as AgentEvent.ToolCall).subagent)
    }

    @Test
    fun aFinishedDispatchDecodesItsSummary() {
        // These are the numbers a collapsed block shows. Dropping them leaves a
        // finished subagent able to say only that it finished.
        val event = AgentEvent.decode(
            """{"ToolUpdate":{"id":"t1","status":"Completed","title":null,"content":null,""" +
                """"diff":null,"locations":[],"subagent":{"agent_type":"general-purpose",""" +
                """"model":"claude-opus-5[1m]","tokens":12479,"tool_uses":1,"duration_ms":4962,""" +
                """"status":"completed"}}}"""
        )
        val summary = (event as AgentEvent.ToolUpdate).subagent!!
        assertEquals("general-purpose", summary.agentType)
        assertEquals(12479L, summary.tokens)
        assertEquals(1L, summary.toolUses)
        assertEquals(4962L, summary.durationMs)
    }

    @Test
    fun aGapDecodesAsAGapAndNotAsNothing() {
        // The contract the whole feature rests on. A decoder that silently
        // skipped an event it did not understand would produce a transcript
        // that is wrong and looks complete.
        val event = AgentEvent.decode("""{"Gap":{"reason":"RingTrimmed"}}""")
        assertEquals(GapReason.RingTrimmed, (event as AgentEvent.Gap).reason)
    }

    @Test
    fun aLoadFailedGapDecodesWithTheAdaptersDetail() {
        // The adapter's own refusal used to reach only a `println!` on the
        // pane's log surface, which chat mode replaces — so it never reached a
        // client at all. It has to survive decoding to reach one.
        val event = AgentEvent.decode(
            """{"Gap":{"reason":{"LoadFailed":{"detail":"permission denied"}}}}"""
        )
        assertEquals(GapReason.LoadFailed("permission denied"), (event as AgentEvent.Gap).reason)
    }

    @Test
    fun aLoadEmptyGapDecodesAsTheUnitVariantItIs() {
        val event = AgentEvent.decode("""{"Gap":{"reason":"LoadEmpty"}}""")
        assertEquals(GapReason.LoadEmpty, (event as AgentEvent.Gap).reason)
    }

    @Test
    fun anEventFromALaterDaemonBecomesAGapRatherThanAThrow() {
        // A client one release behind its daemon must still render the session.
        // Throwing would blank the whole transcript over one unknown event;
        // dropping it would silently shorten it. A gap is the honest third
        // option.
        val event = AgentEvent.decode("""{"SomethingInvented":{"x":1}}""")
        assertEquals(GapReason.Unparsed, (event as AgentEvent.Gap).reason)
    }

    @Test
    fun anUnknownToolStatusIsNotMistakenForStillRunning() {
        // The gap an unknown VARIANT name does not cover: a variant this build
        // knows, carrying an enum value it does not.
        //
        // This used to fall back to PENDING, which the UI reads as still
        // running — see `AgentRows.kt`'s `running` and `Transcript.kt:46`.
        // Every status likely to be invented (cancelled, rejected, timed out)
        // is terminal, so that left a finished tool call spinning forever.
        val event = AgentEvent.decode(
            """{"ToolCall":{"id":"t1","title":"Read","kind":"read","status":"Cancelled","locations":[]}}"""
        )
        val call = event as AgentEvent.ToolCall
        assertEquals(ToolStatus.UNKNOWN, call.status)
    }

    @Test
    fun anUnknownRoleStillShowsWhatWasSaid() {
        // A role is not worth losing a message over: whatever it is called, it
        // came from the agent's side of the conversation.
        val event = AgentEvent.decode("""{"Message":{"role":"Narrator","text":"hello"}}""")
        val message = event as AgentEvent.Message
        assertEquals(Role.AGENT, message.role)
        assertEquals("hello", message.text)
    }

    @Test
    fun unreadableJsonIsAGapRatherThanACrash() {
        // Kotlin's decode does not throw, unlike Swift's, so this is the case
        // that has to be asserted rather than inferred from a `try?` at the
        // call site.
        assertEquals(GapReason.Unparsed, (AgentEvent.decode("not json") as AgentEvent.Gap).reason)
        assertEquals(GapReason.Unparsed, (AgentEvent.decode("{}") as AgentEvent.Gap).reason)
    }

    @Test
    fun aRealSessionStartedFromTheDaemonDecodes() {
        // The exact bytes a live daemon emitted. A decoder that fails here is
        // not a test failure in the abstract: `AgentStream` drops undecodable
        // events, so the chat renders blank — no model selector, no modes, no
        // sign anything is wrong.
        val event = AgentEvent.decode(
            """{"SessionStarted":{"session_id":"s","agent_mode":"default",""" +
                """"available_modes":[{"id":"default","name":"Manual","description":"d"}],""" +
                """"model":"haiku",""" +
                """"available_models":[{"id":"haiku","name":"Haiku","description":"fast"}],""" +
                """"config_options":[{"id":"mode","name":"Mode","description":"","category":"mode",""" +
                """"kind":"select","current_value":"default",""" +
                """"options":[{"id":"default","name":"Manual","description":"d"}]}],""" +
                """"available_commands":[]}}"""
        )
        val started = event as AgentEvent.SessionStarted
        assertEquals("Manual", started.availableModes.first().name)
        assertEquals("haiku", started.model)
        assertEquals("Haiku", started.availableModels.first().name)
        assertEquals("mode", started.configOptions.first().id)
    }

    @Test
    fun aCommandsEventDecodesRatherThanBecomingAGap() {
        // With its description, which the adapter has always sent and which the
        // picker needs — a list of names is a list you have to already know.
        val event = AgentEvent.decode(
            """{"CommandsAvailable":{"commands":[{"id":"init","name":"init","description":"Set up a project"}]}}"""
        )
        assertEquals(
            listOf(AgentChoice("init", "init", "Set up a project")),
            (event as AgentEvent.CommandsAvailable).commands,
        )
    }

    // The gap copy, pinned the way `AgentKitTests` pins the Mac's and the
    // phone's. These sentences are the same sentences on all three platforms,
    // and one gap read on two devices is the only way a drift between them ever
    // shows itself.

    private val everyReason = listOf(
        GapReason.RingTrimmed,
        GapReason.LoadUnsupported,
        GapReason.LoadEmpty,
        GapReason.LoadFailed("permission denied"),
        GapReason.Unparsed,
    )

    @Test
    fun noGapLeaksItsOwnCaseName() {
        // A case name is an internal identifier. The sentence is the whole of
        // what reaches a person.
        for (reason in everyReason) {
            val sentence = reason.sentence
            assertTrue(sentence, sentence.isNotEmpty())
            for (name in listOf("RingTrimmed", "LoadUnsupported", "LoadEmpty", "LoadFailed", "Unparsed")) {
                assertTrue(sentence, !sentence.contains(name))
            }
        }
    }

    @Test
    fun aFailedLoadKeepsItsWordsOutOfTheSentence() {
        val reason = GapReason.LoadFailed("permission denied")
        assertTrue(reason.sentence, !reason.sentence.contains("permission denied"))
        assertTrue(reason.sentence, !reason.sentence.contains(":"))
        assertEquals("permission denied", reason.transcript)
    }

    @Test
    fun onlyAFailedLoadHasAnythingToShow() {
        // Null rather than empty, so a row can ask before it reserves the
        // space: an empty box under a sentence reads as output that failed to
        // arrive.
        assertNull(GapReason.RingTrimmed.transcript)
        assertNull(GapReason.LoadUnsupported.transcript)
        assertNull(GapReason.LoadEmpty.transcript)
        assertNull(GapReason.Unparsed.transcript)
        assertNull(GapReason.LoadFailed("").transcript)
    }

    @Test
    fun anAgentThatCannotReopenReadsDifferentlyFromARefusal() {
        // These two used to be the same sentence, word for word, and the only
        // thing telling them apart was the adapter text spliced onto the end of
        // one of them.
        assertTrue(
            GapReason.LoadUnsupported.sentence != GapReason.LoadFailed("x").sentence
        )
    }

    @Test
    fun onlyAnEmptySessionIsNews() {
        assertTrue(GapReason.LoadEmpty.isInformational)
        assertTrue(!GapReason.RingTrimmed.isInformational)
        assertTrue(!GapReason.LoadFailed("x").isInformational)
    }
}
