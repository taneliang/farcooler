package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The people on a runner and their roles (ov-784), as `member.list` answers, and who gets the picker. */
class RunnerTeamTest {
    private fun reply(you: String, roles: Boolean = true): JsonObject {
        val role = { r: String -> if (roles) ""","role":"$r"""" else "" }
        return Json.parseToJsonElement(
            """{"members":[{"handle":"alice","name":"Alice Chen","rep_panes":[{"terminal":"t","workspace":"w"}]${role("maintainer")}},""" +
                """{"handle":"bob","name":"Bob","rep_panes":[]${role("member")}}],"claude_account":"","you":"$you"}""",
        ).jsonObject
    }

    @Test
    fun eachRoleDecodesAndAnOlderRunnerSaysNone() {
        val team = RunnerTeam.of(reply("bob"))!!
        assertEquals(listOf(RunnerTeam.Role.MAINTAINER, RunnerTeam.Role.MEMBER), team.members.map { it.role })
        assertEquals(1, team.members[0].reps)
        assertNull(RunnerTeam.of(reply("", roles = false))!!.members[0].role)
        assertNull(RunnerTeam.of(JsonObject(emptyMap())))
    }

    @Test
    fun onlyTheOwnerIsOfferedThePickerOnARunnerWithRoles() {
        assertTrue(RunnerTeam.of(reply(""))!!.offersRoles(runnerHasRoles = true))
        assertFalse(RunnerTeam.of(reply(""))!!.offersRoles(runnerHasRoles = false))
        assertFalse(RunnerTeam.of(reply("alice"))!!.offersRoles(runnerHasRoles = true), "a maintainer gives none")
        assertFalse(RunnerTeam.of(reply("(released)"))!!.offersRoles(runnerHasRoles = true))
    }

    @Test
    fun aRemovedDeviceIsToldSoAndOnlyIt() {
        val released = RunnerTeam.of(Json.parseToJsonElement("""{"members":[],"you":"(released)"}""").jsonObject)!!
        assertTrue(released.isReleased)
        assertFalse(released.isOwner)
        assertFalse(released.isShared)
        assertFalse(RunnerTeam.of(reply("bob"))!!.isReleased)
    }

    @Test
    fun aMaintainersRowSaysSoWhereNoPickerDoes() {
        val team = RunnerTeam.of(reply(""))!!
        assertEquals("alice · maintainer · 1 rep", RunnerTeam.detail(team.members[0]))
        assertEquals("alice · 1 rep", RunnerTeam.detail(team.members[0], showsRole = false))
        assertEquals("bob · no rep", RunnerTeam.detail(team.members[1]))
    }

    private fun assertFalse(condition: Boolean, message: String) = org.junit.Assert.assertFalse(message, condition)
}
