package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** Whether members share the runner's logins, per agent (ov-652): the wire's shape and the words. */
class MemberOwnLoginsTest {
    private fun json(text: String) = Json.parseToJsonElement(text) as JsonObject

    @Test
    fun `the switches decode from member list, and a runner before them sends none`() {
        val logins = MemberOwnLogins.of(json("""{"members":[],"you":"","own_logins":{"claude":false,"codex":true,"cursor":false,"may_change":true}}"""))
        assertEquals(MemberOwnLogins(claude = false, codex = true, cursor = false, mayChange = true), logins)
        assertTrue(logins!!.requiresOwn(MemberOwnLogins.Agent.CODEX))
        assertFalse(logins.requiresOwn(MemberOwnLogins.Agent.CURSOR))
        assertNull(MemberOwnLogins.of(json("""{"members":[],"you":""}""")))
    }

    @Test
    fun `each switch says what it means, in sentence case`() {
        assertEquals("Member logins", MemberOwnLogins.SECTION_TITLE)
        assertEquals("Require own Codex account", MemberOwnLogins.toggleTitle(MemberOwnLogins.Agent.CODEX))
        assertEquals("Members who haven’t signed in use this runner’s Claude login.", MemberOwnLogins.stateLine(MemberOwnLogins.Agent.CLAUDE, false))
        assertEquals(
            "Members can’t use Cursor here until they can connect their own account.",
            MemberOwnLogins.stateLine(MemberOwnLogins.Agent.CURSOR, true),
        )
    }

    @Test
    fun `the footer says a shared login is the owner's account, quota and history`() {
        assertEquals(
            "A member on this runner’s login uses your account, its quota and its history, including Claude’s memory, " +
                "Codex sessions and Cursor chats. A change applies the next time each pane starts.",
            MemberOwnLogins.footer(owner = true, mayChange = true),
        )
        assertTrue(MemberOwnLogins.footer(owner = false, mayChange = false).endsWith(" Only this runner’s admins can change these."))
        assertTrue(MemberOwnLogins.footer(owner = false, mayChange = false).contains("the runner owner’s account"))
    }
}
