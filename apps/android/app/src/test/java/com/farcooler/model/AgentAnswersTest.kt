package com.farcooler.model

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The rules a conversation view answers a held ask by (ov-370), as AgentKit's `AgentAnswersTests` holds them. */
class AgentAnswersTest {
    private val color = AgentRow.Ask.Question(
        "Which color?", "Color", listOf(AgentRow.Ask.Option("Red", "Warm"), AgentRow.Ask.Option("Blue", "Calm")), multiSelect = false,
    )
    private val sizes = AgentRow.Ask.Question(
        "Which sizes?", "Sizes", listOf("S", "M", "L").map { AgentRow.Ask.Option(it, "") }, multiSelect = true,
    )

    @Test
    fun onlyAHeldUnansweredAskIsAnswerable() {
        val held = AgentRow.Ask("Question", "Which color?", "AskUserQuestion", 1, false, held = "hook-ask-1")
        assertTrue(AgentConversation.answerable(held))
        assertFalse("answered in the record", AgentConversation.answerable(held.copy(answered = true)))
        assertFalse("the hold ended: the terminal's", AgentConversation.answerable(held.copy(held = null)))
    }

    @Test
    fun answersNeedEveryQuestionAndKeepTheOfferedOrder() {
        val questions = listOf(color, sizes)
        assertNull("sizes unanswered", AgentConversation.answers(questions, mapOf(0 to setOf("Blue")), emptyMap()))
        assertNull("blank Other", AgentConversation.answers(questions, mapOf(0 to setOf("Blue")), mapOf(1 to "  ")))
        assertEquals(
            mapOf("Which color?" to "Blue", "Which sizes?" to "S, L, XL"),
            AgentConversation.answers(questions, mapOf(0 to setOf("Blue"), 1 to setOf("L", "S")), mapOf(1 to "XL")),
        )
        assertEquals(mapOf("Which color?" to "Green"), AgentConversation.answers(listOf(color), emptyMap(), mapOf(0 to "Green")))
        assertEquals(
            "a single-choice question's Other replaces the pick",
            mapOf("Which color?" to "Green"),
            AgentConversation.answers(listOf(color), mapOf(0 to setOf("Red")), mapOf(0 to "Green")),
        )
        assertNull(AgentConversation.answers(emptyList(), emptyMap(), emptyMap()))
    }

    @Test
    fun aPickReplacesOrToggles() {
        assertEquals(setOf("Blue"), AgentConversation.pick("Blue", color, setOf("Red")))
        assertEquals(setOf("S", "M"), AgentConversation.pick("M", sizes, setOf("S")))
        assertEquals(setOf("M"), AgentConversation.pick("S", sizes, setOf("S", "M")))
    }

    @Test
    fun theTitleNamesTheDeviceThatAnswered() {
        val ask = AgentRow.Ask("PlanExit", "# Plan", "ExitPlanMode", 1, false, held = "hook-ask-2")
        assertEquals("Claude has a plan for you to review", AgentConversation.askTitle(ask))
        assertEquals("Answered on iPhone", AgentConversation.askTitle(ask.copy(held = null, answeredBy = "iPhone")))
        assertEquals("Answered", AgentConversation.askTitle(ask.copy(held = null, answered = true)))
    }

    @Test
    fun refusalsAreWords() {
        assertTrue(AgentConversation.answerIssue("not_held").contains("isn’t waiting here"))
        assertEquals("The answer didn’t reach Claude. Answer in the terminal.", AgentConversation.answerIssue("not_delivered"))
        assertEquals("Answer every question first.", AgentConversation.answerIssue("answers"))
        assertFalse("no runner words on screen", AgentConversation.answerIssue("x_y").contains("_"))
    }
}
