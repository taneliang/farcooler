package com.farcooler.model

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** A pane's info says whose Claude login its agent runs on (ov-597). */
class PaneClaudeLoginTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun aMembersLoginIsNamedAfterThem() {
        assertEquals("Uses Alex Rivera’s Claude login", PaneClaudeLogin("alex", "Alex Rivera", false).sentence(null))
    }

    @Test
    fun aMemberWithNoNameIsNamedByTheirHandle() {
        assertEquals("Uses alex’s Claude login", PaneClaudeLogin("alex", " ", false).sentence(null))
    }

    @Test
    fun theOwnersLoginIsTheOwners() {
        assertEquals("Uses the owner’s Claude login", PaneClaudeLogin("", "", true).sentence(null))
    }

    @Test
    fun theViewersOwnLoginIsYours() {
        val alex = PaneClaudeLogin("alex", "Alex Rivera", false)
        val owner = PaneClaudeLogin("", "", true)
        assertEquals("Uses your Claude login", alex.sentence("alex"))
        assertEquals("Uses your Claude login", owner.sentence(""))
        assertEquals("Uses Alex Rivera’s Claude login", alex.sentence(""))
        assertEquals("Uses the owner’s Claude login", owner.sentence("alex"))
    }

    @Test
    fun aReleasedDeviceOwnsNothing() {
        assertEquals("Uses Alex Rivera’s Claude login", PaneClaudeLogin("alex", "Alex Rivera", false).sentence("(released)"))
    }

    @Test
    fun theFleetsKeyDecodesAndAbsentIsNull() {
        val named = json.decodeFromString<Terminal>(
            """{"id":"t1","claudeLogin":{"handle":"alex","name":"Alex Rivera","owner":false}}""",
        )
        assertEquals(PaneClaudeLogin("alex", "Alex Rivera", false), named.claudeLogin)
        assertNull(json.decodeFromString<Terminal>("""{"id":"t1","claudeLogin":null}""").claudeLogin)
        assertNull(json.decodeFromString<Terminal>("""{"id":"t1"}""").claudeLogin)
    }
}
