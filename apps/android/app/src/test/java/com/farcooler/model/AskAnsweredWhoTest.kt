package com.farcooler.model

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** An answered ask says who answered on a runner with members (ov-746), as AgentKit's `AskAnsweredWhoTests` hold it. */
class AskAnsweredWhoTest {
    private fun person(handle: String, name: String) =
        JsonObject(mapOf("handle" to JsonPrimitive(handle), "name" to JsonPrimitive(name)))

    private val bob = person("bob", "Bob")
    private val alice = person("alice", "Alice")
    private val owner = person("", "")

    /** The ask as the wire decodes it, so the fields' names are held too. */
    private fun ask(who: JsonObject? = null, forWho: JsonObject? = null): AgentRow.Ask {
        var body: Map<String, kotlinx.serialization.json.JsonElement> = mapOf(
            "kind" to JsonPrimitive("Question"), "text" to JsonPrimitive("Which color?"), "tool" to JsonPrimitive("AskUserQuestion"),
            "asked_ms" to JsonPrimitive(1), "answered" to JsonPrimitive(true), "answered_by" to JsonPrimitive("iPhone"),
        )
        if (who != null) body = body + ("answered_person" to who)
        if (forWho != null) body = body + ("answered_for" to forWho)
        val row = JsonObject(mapOf(
            "id" to JsonPrimitive("ask:1"), "ord" to JsonPrimitive(0), "rev" to JsonPrimitive(1), "provisional" to JsonPrimitive(false),
            "kind" to JsonObject(mapOf("Ask" to JsonObject(body))),
        ))
        return (AgentRowJson.row(row)!!.kind as AgentRow.Kind.OfAsk).ask
    }

    @Test
    fun aSoloRunnerNamesNoOneSoTheTitleIsTheDevices() {
        val solo = ask()
        assertNull(solo.answeredPerson)
        assertEquals("Answered on iPhone", AgentConversation.askTitle(solo, viewer = ""))
        assertEquals("Answered on iPhone", AgentConversation.askTitle(solo))
    }

    @Test
    fun whoAnsweredForYouAndForOthers() {
        assertEquals("You answered", AgentConversation.askTitle(ask(bob), viewer = "bob"))
        assertEquals("Bob answered", AgentConversation.askTitle(ask(bob), viewer = ""))
        assertEquals("Bob answered for Alice", AgentConversation.askTitle(ask(bob, alice), viewer = ""))
        assertEquals("Alice answered for you", AgentConversation.askTitle(ask(alice, bob), viewer = "bob"))
        assertEquals("You answered for Bob", AgentConversation.askTitle(ask(alice, bob), viewer = "alice"))
        assertEquals("Alice answered for the owner", AgentConversation.askTitle(ask(alice, owner), viewer = "bob"))
    }

    @Test
    fun theOwnerIsTheEmptyHandle() {
        assertEquals("You answered", AgentConversation.askTitle(ask(owner), viewer = ""))
        assertEquals("The owner answered", AgentConversation.askTitle(ask(owner), viewer = "bob"))
        assertEquals("The owner answered for you", AgentConversation.askTitle(ask(owner, bob), viewer = "bob"))
    }

    @Test
    fun aDeviceNotToldWhoItIsAndAReleasedOneNeverGetAYou() {
        assertEquals("Answered on iPhone", AgentConversation.askTitle(ask(bob), viewer = null))
        assertEquals("The owner answered", AgentConversation.askTitle(ask(owner), viewer = "(released)"))
    }

    @Test
    fun aMemberWithNoNameIsSaidByHandle() {
        assertEquals("bob answered", AgentConversation.askTitle(ask(person("bob", "")), viewer = ""))
    }

    @Test
    fun aHeldAskStillWaitingReadsAsTheQuestion() {
        val waiting = ask(bob).copy(answered = false, held = "hook-ask-1")
        assertEquals("Claude is asking a question", AgentConversation.askTitle(waiting, viewer = ""))
    }

    @Test
    fun aNameIsUsedExactlyAsSent() {
        val berg = person("vdb", "van der Berg")
        assertEquals("van der Berg answered", AgentConversation.askTitle(ask(berg), viewer = ""))
        assertEquals("Bob answered for van der Berg", AgentConversation.askTitle(ask(bob, berg), viewer = ""))
    }

    @Test
    fun anAnswerForTheAnswerersOwnAskDropsTheForClause() {
        assertEquals("Bob answered", AgentConversation.askTitle(ask(bob, bob), viewer = ""))
        assertEquals("You answered", AgentConversation.askTitle(ask(bob, bob), viewer = "bob"))
        assertEquals("The owner answered", AgentConversation.askTitle(ask(owner, owner), viewer = "bob"))
    }
}
