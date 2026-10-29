package com.farcooler.ceremony

import com.farcooler.model.PeopleAccess
import com.farcooler.model.PersonAccess
import com.farcooler.model.StepUpReason
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Add Teammate with a person and an access (ov-856), the Access menu and Limit to Viewer: what each caller is offered,
 * what is sent, and what a refusal says. AgentKit's `TeammateAccessTests`, in Kotlin: the same runner, the same
 * answers.
 */
class TeammateAccessTest {
    private fun members(you: String): JsonObject = Json.parseToJsonElement(
        """{"members":[
          {"handle":"dan","name":"Dan","devices":["phone-d"],"access":"admin","account":"user_dan"},
          {"handle":"bob","name":"Bob","devices":["phone-b"],"access":"member","account":"user_bob"},
          {"handle":"cy","name":"Cy","devices":[],"access":"viewer","account":""}
        ],"claude_account":"","you":"$you",
        "device_access":[
          {"client_id":"phone-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},
          {"client_id":"phone-d","person":"dan","access":"admin","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},
          {"client_id":"phone-b","person":"bob","access":"member","cap":"","check":false,"step_up":"","step_up_code":"","step_up_at":0}
        ]}""",
    ).jsonObject

    private fun facts(you: String, device: String, account: String = "user_bob") =
        TeammateRunnerFacts.of(members(you), account, device)!!

    private fun choice(access: PersonAccess = PersonAccess.MEMBER, person: String? = null) = TeammateChoice(
        who = CeremonyTeammate("user_eve", "eve.jones@example.com", "Platform"), directory = "{}", grantedBy = "alice@example.com",
        access = access, person = person,
    )

    private fun JsonObject.text(key: String) = (this[key] as JsonPrimitive).content

    @Test
    fun theOwnerAnAdminAndAMemberAreToldApartWithThePeopleTheDeviceCouldJoin() {
        assertEquals(PersonAccess.OWNER, facts("", "phone-o").caller)
        val admin = facts("dan", "phone-d")
        assertEquals(PersonAccess.ADMIN, admin.caller)
        assertTrue(admin.signs)
        assertEquals(PersonAccess.MEMBER, facts("bob", "phone-b").caller)
        assertEquals(listOf("bob", "cy"), admin.candidates.map { it.handle })
        assertEquals("bob", admin.matching)
        assertEquals(setOf("dan", "bob", "cy"), admin.taken)
        assertNull(facts("", "phone-o", account = "user_new").matching)
    }

    @Test
    fun onlyAnOwnerOrAnAdminCanTickARunnerAndTheSentenceSaysWho() {
        assertNull(TeammateAccessRules.blocked(facts("", "phone-o"), "studio"))
        assertEquals("Only an Owner or an Admin can add people to studio.", TeammateAccessRules.blocked(facts("bob", "phone-b"), "studio"))
        assertEquals("Update Far Cooler on studio to add people from here.", TeammateAccessRules.blocked(null, "studio"))
    }

    @Test
    fun adminIsOfferedOnlyWhenTheOwnerIsAddingOnEveryRunner() {
        assertEquals(listOf(PersonAccess.ADMIN, PersonAccess.MEMBER, PersonAccess.VIEWER), TeammateAccessRules.choices(listOf(PersonAccess.OWNER, PersonAccess.OWNER)))
        assertEquals(listOf(PersonAccess.MEMBER, PersonAccess.VIEWER), TeammateAccessRules.choices(listOf(PersonAccess.OWNER, PersonAccess.ADMIN)))
        assertEquals(listOf(PersonAccess.MEMBER, PersonAccess.VIEWER), TeammateAccessRules.choices(listOf(PersonAccess.ADMIN)))
    }

    @Test
    fun aNewPersonIsSentWithTheAccessChosenAndAHandleNoOneHasAndViewerIsAReadLine() {
        val facts = TeammateRunnerFacts(caller = PersonAccess.OWNER, taken = setOf("eve-jones"))
        val args = TeammateAccessRules.arguments("KEY", "Eve’s Pixel", "phone-e", "N", choice(PersonAccess.VIEWER), facts)
        val person = args["teammate"]!!.jsonObject
        assertEquals("eve-jones-2", person.text("handle"))
        assertEquals("viewer", person.text("access"))
        assertEquals("user_eve", person.text("account"))
        assertEquals("read", args.text("scope"))
        assertEquals("true", args.text("forTeammate"))
        val admin = TeammateAccessRules.arguments("KEY", "x", "p", "", choice(PersonAccess.ADMIN), facts)
        assertEquals("an Admin's line is control, never host_admin", "control", admin.text("scope"))
        assertEquals("admin", admin["teammate"]!!.jsonObject.text("access"))
        assertNull(args["shellAccess"])
    }

    @Test
    fun anExistingPersonIsSentByTheirHandleWithTheAccessTheyHave() {
        val facts = facts("", "phone-o", account = "user_bob")
        val args = TeammateAccessRules.arguments("K", "x", "p", "", choice(PersonAccess.ADMIN, person = "bob"), facts)
        val person = args["teammate"]!!.jsonObject
        assertEquals("bob", person.text("handle"))
        assertEquals("Bob", person.text("name"))
        assertEquals("they keep what they have, whatever the menu said", "member", person.text("access"))
        assertEquals("control", args.text("scope"))
    }

    @Test
    fun thePromptAndTheRefusalsAreAppleStyleAndTheAddLimitSaysFiveADay() {
        assertEquals("Add Eve’s Pixel to studio as a Member.", TeammateAccessRules.reason("Eve’s Pixel", "studio", "member"))
        assertEquals("Add Eve’s Pixel to studio as an Admin.", TeammateAccessRules.reason("Eve’s Pixel", "studio", "admin"))
        assertEquals("You can add five devices a day from this phone. Try again tomorrow.", TeammateAccessRules.refusal("add_limit", "studio", "phone"))
        assertEquals("Only the Owner can make someone an Admin.", TeammateAccessRules.refusal("admin_is_owners", "studio", "phone"))
        assertEquals("Only an Owner or an Admin can add people to studio.", TeammateAccessRules.refusal("not_admin", "studio", "phone"))
        assertEquals("Limit Cy’s Pixel on studio to Viewer.", StepUpReason.limit("Cy’s Pixel", "studio"))
        assertEquals(
            "Bob is already on studio as a Member, and keeps that access.",
            TeammateAccessRules.keeps(ExistingPerson("bob", "Bob", PersonAccess.MEMBER), "studio"),
        )
        for (word in listOf("add_limit", "admin_is_owners", "access_exists", "handle_taken", "scope_access", "not_admin")) {
            val said = TeammateAccessRules.refusal(word, "studio", "phone")
            assertFalse("$word: $said", said.contains("_") || said.contains("Err("))
        }
    }

    @Test
    fun theAccessMenuOffersWhatTheRunnerWouldAllowByWhoIsAsking() {
        val owner = PeopleAccess.of(members(""), null, "phone-o")!!
        val dan = owner.people.first { it.handle == "dan" }
        val bob = owner.people.first { it.handle == "bob" }
        assertEquals(listOf(PersonAccess.ADMIN, PersonAccess.MEMBER, PersonAccess.VIEWER), owner.accessChoices(dan))
        assertTrue("nobody changes the Owner", owner.accessChoices(owner.people[0]).isEmpty())

        val admin = PeopleAccess.of(members("dan"), null, "phone-d")!!
        assertEquals(listOf(PersonAccess.MEMBER, PersonAccess.VIEWER), admin.accessChoices(admin.people.first { it.handle == "bob" }))
        assertEquals("themselves, down", listOf(PersonAccess.MEMBER, PersonAccess.VIEWER), admin.accessChoices(admin.people.first { it.handle == "dan" }))
        assertTrue(admin.accessChoices(admin.people[0]).isEmpty())

        val member = PeopleAccess.of(members("bob"), null, "phone-b")!!
        assertTrue(member.accessChoices(dan).isEmpty() && member.accessChoices(bob).isEmpty())
    }

    @Test
    fun limitToViewerIsOfferedOnADeviceTheCallerManages() {
        val admin = PeopleAccess.of(members("dan"), null, "phone-d")!!
        fun device(board: PeopleAccess, id: String) = board.people.flatMap { it.devices }.first { it.id == id }
        assertTrue("a Member's", admin.mayLimit(device(admin, "phone-b")))
        assertTrue("their own", admin.mayLimit(device(admin, "phone-d")))
        assertFalse("the Owner's", admin.mayLimit(device(admin, "phone-o")))
        val owner = PeopleAccess.of(members(""), null, "phone-o")!!
        assertTrue(owner.mayLimit(device(owner, "phone-d")))
        val member = PeopleAccess.of(members("bob"), null, "phone-b")!!
        assertFalse(member.mayLimit(device(member, "phone-b")))
    }
}
