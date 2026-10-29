package com.farcooler.model

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * The People screen's rows (ov-856, §8): each person with their Access, their devices with who added them, a Viewer
 * limit, a shell warning, and the two sections that appear only when they have something in them. AgentKit's
 * `PeopleAccessTests`, in Kotlin: the same runner, the same answers.
 */
class PeopleAccessTest {
    private val members = Json.parseToJsonElement(
        """{"members":[
          {"handle":"cosmo","name":"Cosmo","devices":["phone-c","pad-c"],"access":"member"},
          {"handle":"dan","name":"Dan","devices":["phone-d","mac-d"],"access":"admin"}
        ],"claude_account":"","you":"",
        "device_access":[
          {"client_id":"mac-o","person":"","access":"owner","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},
          {"client_id":"legacy-t","person":"","access":"owner","cap":"member","check":true,"step_up":"","step_up_code":"","step_up_at":0},
          {"client_id":"phone-c","person":"cosmo","access":"member","cap":"","check":false,"step_up":"trusted","step_up_code":"","step_up_at":5},
          {"client_id":"pad-c","person":"cosmo","access":"member","cap":"viewer","check":false,"step_up":"","step_up_code":"","step_up_at":0},
          {"client_id":"phone-d","person":"dan","access":"admin","cap":"","check":false,"step_up":"proposed","step_up_code":"418207","step_up_at":9},
          {"client_id":"mac-d","person":"dan","access":"admin","cap":"","check":false,"step_up":"expired","step_up_code":"","step_up_at":1}
        ]}""",
    ).jsonObject

    private val clients = Json.parseToJsonElement(
        """{"clients":[
          {"clientId":"phone-c","label":"farcooler-Cosmos-iPhone-t7xq9vd8","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"pad-c","label":"farcooler-iPad-abcd1234","shellAccess":false,"foreign":false,"addedBy":"phone-c"},
          {"clientId":"phone-d","label":"farcooler-iPhone-k3k3k3k3","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"mac-d","label":"farcooler-Dans-Mac-zzzz9999","shellAccess":false,"foreign":false,"addedBy":""},
          {"clientId":"mac-d","label":"farcooler-shell-Dans-Mac-zzzz9999.mac-d","shellAccess":true,"foreign":false,"addedBy":""},
          {"clientId":"","label":"someone@laptop","shellAccess":false,"foreign":true,"addedBy":""}
        ]}""",
    ).jsonObject

    private fun board(you: String = "") = PeopleAccess.of(members, clients, you)!!

    @Test
    fun eachPersonSaysTheirAccessAndTheOwnerComesFirst() {
        val people = board().people
        assertEquals(listOf(PersonAccess.OWNER, PersonAccess.MEMBER, PersonAccess.ADMIN), people.map { it.access })
        assertEquals(listOf("Owner", "Cosmo", "Dan"), people.map { it.name })
        assertTrue(people[0].isYou && !people[1].isYou)
    }

    @Test
    fun aDeviceIsNamedAsAPersonWouldAndSaysWhoAddedIt() {
        val cosmo = board().people.first { it.handle == "cosmo" }
        assertEquals(listOf("Cosmos-iPhone", "iPad"), cosmo.devices.map { it.name })
        assertNull(cosmo.devices[0].addedBy)
        assertEquals("Added by Cosmos-iPhone", cosmo.devices[1].addedBy)
    }

    @Test
    fun aDeviceHeldAtViewerWearsTheBadge() {
        val cosmo = board().people.first { it.handle == "cosmo" }
        assertEquals(listOf(false, true), cosmo.devices.map { it.viewerOnThisDevice })
    }

    @Test
    fun aPlainLineBesideADevicesOwnIsAShellAndOnlyThatDeviceHasIt() {
        val dan = board().people.first { it.handle == "dan" }
        assertEquals(listOf(false, true), dan.devices.map { it.hasShell })
        assertEquals("This device has a shell on studio, so it can do everything.", PeopleAccess.shellWarning("studio"))
    }

    @Test
    fun removingADeviceCountsWhatItAddedAndThoseTheyAdded() {
        val cosmo = board().people.first { it.handle == "cosmo" }
        assertEquals(listOf(2, 1), cosmo.devices.map { it.removal })
        val chain = PeopleAccess.addedCounts(mapOf("b" to "a", "c" to "b", "d" to "b"))
        assertEquals(mapOf("a" to 3, "b" to 2), chain)
        assertEquals(emptyMap<String, Int>(), PeopleAccess.addedCounts(mapOf("a" to "b", "b" to "a")).filterKeys { false })
    }

    @Test
    fun theTwoSectionsHoldExactlyTheDevicesThatBelongInThem() {
        val b = board()
        assertEquals(listOf("legacy-t"), b.toCheck.map { it.id })
        assertEquals("an expired proposal is not waiting: the device has to ask again", listOf("phone-d"), b.waiting.map { it.id })
        assertEquals("418207", b.waiting[0].code)
    }

    @Test
    fun aDeviceMayApproveAnotherOfTheSamePersonsOnceItHasATrustedKey() {
        val b = board("phone-c")
        val theirs = b.waiting.first()
        assertFalse("Cosmo's phone can't approve Dan's", b.mayApprove(theirs, b.thisDevice))
        val dan = board("mac-d")
        assertFalse("an expired key doesn't sign", dan.mayApprove(theirs, dan.thisDevice))
        val sameDan = theirs.copy(id = "phone-d2", stepUp = StepUpKeyStanding.TRUSTED)
        assertTrue(b.mayApprove(theirs, sameDan))
        assertFalse(b.mayApprove(theirs, null))
    }

    @Test
    fun theApprovalSheetReadsTheCodeInTwoHalvesAndSaysToHoldTheDevice() {
        val waiting = board().waiting.first()
        assertEquals("418 207", PeopleAccess.spoken("418207"))
        assertEquals("Only allow this if you’re holding iPhone right now. Check that it shows 418 207.", PeopleAccess.approvalPrompt(waiting))
    }

    @Test
    fun whatAMenuOffersDependsOnWhoIsChoosing() {
        assertEquals(listOf(PersonAccess.ADMIN, PersonAccess.MEMBER, PersonAccess.VIEWER), PersonAccess.choices(PersonAccess.OWNER))
        assertEquals("an Admin never makes an Admin", listOf(PersonAccess.MEMBER, PersonAccess.VIEWER), PersonAccess.choices(PersonAccess.ADMIN))
        assertTrue(PersonAccess.choices(PersonAccess.MEMBER).isEmpty())
        assertEquals("Can add and remove people and their devices.", PersonAccess.ADMIN.summary)
    }

    @Test
    fun aRunnerBeforePersonAccessHasNoAccessToShow() {
        val old = PeopleAccess.of(Json.parseToJsonElement("""{"members":[{"handle":"a","name":"A","devices":["x"]}],"claude_account":"","you":"a"}""").jsonObject)!!
        assertFalse(old.recorded)
        assertTrue(old.toCheck.isEmpty() && old.waiting.isEmpty())
        assertEquals(listOf("A"), old.people.map { it.name })
        assertNull(PeopleAccess.of(JsonObject(emptyMap())))
        assertNotNull(PeopleAccess.of(members))
    }

    @Test
    fun theFileNamesAreNotWhatAPersonSees() {
        assertEquals("iPhone", PeopleAccess.tidy("farcooler-iPhone-t7xq9vd8"))
        assertEquals("Mac", PeopleAccess.tidy("farcooler-shell-Mac-zzzz9999"))
        assertEquals("laptop", PeopleAccess.tidy("laptop"))
    }
}
