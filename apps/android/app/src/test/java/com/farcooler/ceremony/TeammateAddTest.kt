package com.farcooler.ceremony

import com.farcooler.account.RelayAnswer
import kotlinx.coroutines.test.runTest
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Add Teammate from an Android phone (ov-854): what the phone sends, what it
 * reads back, and what it says about each runner. The iPhone's
 * `TeammateAddTests` pins the same words.
 */
class TeammateAddTest {
    private fun choice(scope: TeammateScope = TeammateScope.CONTROL, handle: String? = null, name: String? = null) =
        TeammateChoice(
            who = CeremonyTeammate("user_bob", "bob.smith@example.com", "Platform"),
            directory = "{}",
            grantedBy = "alice@example.com",
            scope = scope,
            handle = handle ?: "bob-smith",
            name = name ?: "Bob Smith",
        )

    // What the phone sends

    @Test
    fun theCallIsOneTeammateEnrollWithTheChosenScopeAndTheDirectorysAccount() {
        assertEquals("a plain client.enroll would leave the key the owner's", "teammate.enroll", TeammateAddRequest.METHOD)
        val arguments = TeammateAddRequest.arguments(
            "ssh-ed25519 AAAA bob", "Bob’s Pixel", "phone-bob", "NODE", choice(TeammateScope.READ, name = "  Bob Smith  "),
        )
        assertEquals("read", arguments["scope"])
        assertEquals("bob-smith", arguments["handle"])
        assertEquals("Bob Smith", arguments["name"])
        assertEquals("user_bob", arguments["account"])
        assertEquals("bob.smith@example.com", arguments["email"])
        assertEquals("phone-bob", arguments["clientId"])
        assertEquals("NODE", arguments["nodeKey"])
        assertNull("a teammate never gets a shell line", arguments["shellAccess"])
    }

    @Test
    fun accessIsControlOrReadAndNothingWider() {
        assertEquals(listOf("control", "read"), TeammateScope.entries.map { it.word })
        assertEquals(listOf("Control", "Read"), TeammateScope.entries.map { it.title })
        assertEquals("Control is the default", TeammateScope.CONTROL, TeammateChoice(
            CeremonyTeammate("a", "a@example.com", "T"), "", "x@example.com",
        ).scope)
    }

    // Who they are

    @Test
    fun aSuggestedHandleAndNameComeFromTheEmailAndAnInvalidHandleIsNotComplete() {
        val suggested = TeammateChoice(CeremonyTeammate("user_bob", "bob.smith@example.com", "Platform"), "", "a@example.com")
        assertEquals("bob-smith", suggested.handle)
        assertEquals("Bob Smith", suggested.name)
        assertTrue(suggested.isComplete)
        assertFalse(choice(handle = "Bob").isComplete)
        assertFalse("a handle the runner keeps for itself", choice(handle = "owner").isComplete)
        assertFalse(choice(name = "   ").isComplete)
    }

    @Test
    fun namesAreSuggestedTheWayTheMacSuggestsThem() {
        assertEquals("bob-smith", TeammateNames.handle("bob.smith@example.com"))
        assertEquals("alex-2-b", TeammateNames.handle("alex_2+b@example.com"))
        assertEquals("teammate", TeammateNames.handle("123@example.com"))
        assertEquals("digits-dropped", TeammateNames.handle("42digits.dropped@example.com"))
        assertEquals("owner-2", TeammateNames.handle("owner@example.com"))
        assertEquals("Bob Smith", TeammateNames.name("bob.smith@example.com"))
        assertEquals("Alex B", TeammateNames.name("alex_2+b@example.com"))
        assertEquals("123@example.com", TeammateNames.name("123@example.com"))
        assertEquals(64, TeammateNames.name("a".repeat(100) + "@example.com").length)
        assertEquals(32, TeammateNames.handle("a".repeat(40) + "@example.com").length)
    }

    // What the phone reads back

    @Test
    fun eachOutcomeWordTheClientCoreAnswersDecodesAndStillThereSurvives() {
        fun decode(json: String) = TeammateAddOutcome.decode(json)
        assertEquals(TeammateAddOutcome.Written("tok"), decode("""{"outcome":"written","tunnel":"tok"}"""))
        assertEquals(TeammateAddOutcome.Written(""), decode("""{"outcome":"written"}"""))
        assertEquals(TeammateAddOutcome.NeedsAdmin, decode("""{"outcome":"needs_admin"}"""))
        assertEquals(TeammateAddOutcome.TooOld, decode("""{"outcome":"too_old"}"""))
        assertEquals(TeammateAddOutcome.CouldNotRead, decode("""{"outcome":"could_not_read"}"""))
        assertEquals(TeammateAddOutcome.SomeoneElses, decode("""{"outcome":"someone_elses"}"""))
        assertEquals(TeammateAddOutcome.NotAdded(true), decode("""{"outcome":"not_added","stillThere":true}"""))
        assertEquals(TeammateAddOutcome.NotAdded(false), decode("""{"outcome":"not_added","stillThere":false}"""))
        assertEquals(TeammateAddOutcome.NotRecorded(true), decode("""{"outcome":"not_recorded","stillThere":true}"""))
        assertEquals(TeammateAddOutcome.NotUpdated, decode("""{"outcome":"not_updated"}"""))
    }

    @Test
    fun anAnswerThatDoesntReadOrNamesAWordThisBuildLacksIsNeverASuccess() {
        for (json in listOf("", "nope", "{}", """{"outcome":"written_somehow"}""", """{"outcome":3}""")) {
            val outcome = TeammateAddOutcome.decode(json)
            assertEquals(json, TeammateAddOutcome.NotReached, outcome)
            assertFalse(outcome.isWritten)
        }
        assertEquals(TeammateAddOutcome.NotReached, TeammateAddOutcome.decode(null as JsonObject?))
        assertTrue(TeammateAddOutcome.Written("").isWritten)
    }

    @Test
    fun aReplyCarriesWhyARunnerIsPending() {
        assertNull(TeammateAddOutcome.Written("").pendingWhy)
        assertEquals("too_old", TeammateAddOutcome.TooOld.pendingWhy)
        assertEquals("not_allowed", TeammateAddOutcome.NeedsAdmin.pendingWhy)
        assertEquals("not_connected", TeammateAddOutcome.NotReached.pendingWhy)
        assertEquals("failed", TeammateAddOutcome.NotRecorded(false).pendingWhy)
    }

    // What the phone says

    @Test
    fun aBoundRunnerIsNotMentionedAndEachOtherOneIsByNameWithWhatToDo() {
        assertNull(
            TeammateAddCopy.note(
                listOf("Cosmo" to TeammateAddOutcome.Written(""), "Atlas" to TeammateAddOutcome.Written("t")), "phone",
            ),
        )
        val note = TeammateAddCopy.note(
            listOf(
                "Cosmo" to TeammateAddOutcome.Written(""),
                "Atlas" to TeammateAddOutcome.NeedsAdmin,
                "Borealis" to TeammateAddOutcome.TooOld,
            ),
            "phone",
        )!!
        assertFalse(note.contains("Cosmo"))
        assertTrue(note.contains("This phone can only control Atlas, so it can’t add a teammate there. Add your teammate from your Mac."))
        assertTrue(note.contains("Borealis’s Far Cooler is too old to record whose device this is"))
    }

    @Test
    fun aKeyThatCouldntBeTakenOffAgainIsSaidToBeStillThere() {
        val gone = TeammateAddCopy.sentence(TeammateAddOutcome.NotRecorded(false), "Cosmo", "phone")!!
        val there = TeammateAddCopy.sentence(TeammateAddOutcome.NotRecorded(true), "Cosmo", "phone")!!
        assertTrue(gone.contains("took the device’s key off again, so it has no access there"))
        assertTrue(there.contains("couldn’t make sure the device’s key isn’t on that runner"))
        assertFalse(there.contains("so it has no access there"))
        assertTrue(gone.startsWith("Far Cooler couldn’t record this device as your teammate’s on Cosmo."))
    }

    @Test
    fun everySentenceIsASentenceWithNoRustWordInIt() {
        val outcomes = listOf(
            TeammateAddOutcome.NeedsAdmin, TeammateAddOutcome.TooOld, TeammateAddOutcome.CouldNotRead,
            TeammateAddOutcome.SomeoneElses, TeammateAddOutcome.NotAdded(true), TeammateAddOutcome.NotAdded(false),
            TeammateAddOutcome.NotRecorded(true), TeammateAddOutcome.NotRecorded(false), TeammateAddOutcome.NotUpdated,
            TeammateAddOutcome.NotReached,
        )
        for (outcome in outcomes) {
            val sentence = TeammateAddCopy.sentence(outcome, "Cosmo", "phone")
            assertNotNull("$outcome", sentence)
            assertTrue("$outcome", sentence!!.contains("Cosmo"))
            assertTrue("$outcome", sentence.endsWith("."))
            for (word in listOf("needs_admin", "too_old", "Err(", "error:", "host_admin", "member.set")) {
                assertFalse("$outcome leaks $word", sentence.contains(word))
            }
        }
    }

    @Test
    fun theRefusalsATeammatesCodeCanMeetNameWhatToDoNext() {
        assertTrue(TeammateRefusalCopy.notATeammate("alice@example.com").contains("sign in to alice@example.com"))
        assertTrue(TeammateRefusalCopy.HELD_KEY_ON_A_PHONE.contains("Add it from your Mac"))
        assertTrue(TeammateRefusalCopy.DIRECTORY_UNREACHABLE.contains("Check your connection"))
        assertTrue(TeammateRefusalCopy.UNREGISTERED_KEY.contains("sign in to Far Cooler on it"))
        // The phone shows the same sentences the core's words map to.
        assertEquals(Refusal.HeldKey, Refusal.of("held_key", null, null))
        assertEquals("Couldn’t check your teams", Refusal.of("directory_unreachable", null, null).title)
        assertEquals(Refusal.NotATeammate(""), Refusal.of("not_a_teammate", null, null))
    }

    // The directory

    private fun me(vararg teams: Pair<String, Boolean?>): JsonObject = buildJsonObject {
        put("user", buildJsonObject { put("id", JsonPrimitive("user_alice")) })
        put(
            "teams",
            JsonArray(
                teams.map { (id, personal) ->
                    buildJsonObject {
                        put("id", JsonPrimitive(id))
                        if (personal != null) put("personal", JsonPrimitive(personal))
                    }
                },
            ),
        )
    }

    @Test
    fun theDirectorysAnswersTravelAsTheyCameMeThenEachSharedTeamsRosterNeverThePersonalOnes() = runTest {
        val asked = mutableListOf<String>()
        val answer = TeamDirectory.fetch { path, body ->
            asked += if (path == TeamDirectory.MEMBERS) "members:${(body["team"] as JsonPrimitive).content}" else path
            when {
                path == TeamDirectory.ME -> RelayAnswer.Answered(me("t-personal" to true, "t-platform" to false, "t-finance" to false))
                body["team"].toString() == "\"t-platform\"" ->
                    RelayAnswer.Answered(buildJsonObject { put("members", JsonArray(emptyList())) })
                // Finance refuses a billing member's roster, and it's left out
                // rather than failing the whole answer.
                else -> RelayAnswer.Refused(403, null)
            }
        }
        assertEquals(listOf("/v1/directory/me", "members:t-platform", "members:t-finance"), asked)
        val parsed = kotlinx.serialization.json.Json.parseToJsonElement(answer!!).jsonObject
        assertEquals(1, parsed["rosters"]!!.jsonArray.size)
        assertNotNull(parsed["me"]!!.jsonObject["user"])
    }

    @Test
    fun aDirectoryThatCantBeAskedIsNullNeverAnEmptyYes() = runTest {
        assertNull(TeamDirectory.fetch { _, _ -> RelayAnswer.Unreachable("offline") })
        assertNull(TeamDirectory.fetch { _, _ -> RelayAnswer.Refused(500, null) })
    }
}
