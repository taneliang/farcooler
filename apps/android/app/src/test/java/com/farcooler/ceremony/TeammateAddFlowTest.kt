package com.farcooler.ceremony

import com.farcooler.data.Reach
import com.farcooler.model.PersonAccess
import com.farcooler.data.Runner
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Add Teammate through the ceremony store (ov-854): a code from another
 * account is held while the directory is asked, opens a teammate's
 * confirmation only when the core vouches for it, writes only on the tap, at
 * the scope chosen and only on the runners ticked, and vouches for nothing. The
 * core, the clock and the runners are answered by fakes.
 */
class TeammateAddFlowTest {
    private val keyA = "ssh-ed25519 AAAAbob farcooler-Pixel"

    private fun offer(account: String) =
        """{"v":2,"key_a":"$keyA","name":"Bob’s Pixel","account":"$account","channel":"stable","ceremony":"c-1","node_key":""}"""

    private class Core : CeremonyCalls {
        var scanTeammateAnswer: CeremonyCore.Answer = CeremonyCore.Answer.Payload(
            """{"offer":"{\"v\":2,\"key_a\":\"ssh-ed25519 AAAAbob farcooler-Pixel\",\"name\":\"Bob’s Pixel\",\"account\":\"user_bob\",\"channel\":\"stable\",\"ceremony\":\"c-1\",\"node_key\":\"\"}","teammate":{"account":"user_bob","email":"bob.smith@example.com","team":"Platform"},"held":[]}""",
        )
        val directories = mutableListOf<String?>()
        val repliesGrantedBy = mutableListOf<String>()
        var replyAnswer: CeremonyCore.Answer = CeremonyCore.Answer.Payload("""{"runners":[]}""")

        override fun scan(encoded: String, expectingAccount: String, heldMs: Long): CeremonyCore.Answer =
            if (encoded.contains(""""account":"$expectingAccount"""")) {
                CeremonyCore.Answer.Payload(encoded)
            } else {
                CeremonyCore.Answer.Refused(Refusal.WrongAccount)
            }

        override fun scanTeammate(encoded: String, expectingAccount: String, directory: String?, heldMs: Long): CeremonyCore.Answer {
            directories += directory
            return if (directory == null) CeremonyCore.Answer.Refused(Refusal.DirectoryUnreachable) else scanTeammateAnswer
        }

        override fun reply(offerJson: String, runners: List<CeremonyRunner>) = error("a teammate's reply is not an owner's")
        override fun teammateReply(offerJson: String, runners: List<CeremonyRunner>, grantedBy: String): CeremonyCore.Answer {
            repliesGrantedBy += grantedBy
            return replyAnswer
        }
        override fun fingerprint(publicKey: String) = "SHA256:test"
        override fun clientId(publicKey: String) = "farcooler-0123456789ab"
    }

    private val buildBox = Runner(id = "r1", label = "build-box", reach = Reach.Direct("build-box.example.com", 22), user = "alice")
    private val gpu = Runner(id = "r2", label = "gpu-workstation", reach = Reach.Direct("gpu.example.com", 22), user = "alice")

    private data class Asked(val keyA: String, val clientId: String, val teammate: TeammateChoice, val runners: List<String>)

    private class Recorder(var outcome: TeammateAddOutcome = TeammateAddOutcome.Written("")) {
        val asked = mutableListOf<Asked>()
        val enroller = TeammateEnroller { publicKey, _, clientId, _, teammate, runners, _ ->
            asked += Asked(publicKey, clientId, teammate, runners.map { it.label })
            runners.associate { it.id to outcome }
        }
    }

    private fun store(
        core: Core,
        directory: String? = "{}",
        facts: TeammateRunnerFacts = TeammateRunnerFacts(caller = PersonAccess.OWNER, taken = setOf("dan")),
    ): CeremonyStore {
        val store = CeremonyStore(account = "user_alice", accountEmail = "alice@example.com", deviceName = "Alice’s Pixel")
        store.core = core
        store.clock = { 1_000L }
        store.teamDirectory = { directory }
        store.teammateFacts = { runner, _ ->
            if (runner.id == gpu.id) TeammateRunnerFacts(caller = PersonAccess.MEMBER) else facts
        }
        store.certify = { _, _ -> error("a teammate's device is never this phone's to vouch for") }
        return store
    }

    private suspend fun scanned(
        core: Core = Core(),
        directory: String? = "{}",
        facts: TeammateRunnerFacts = TeammateRunnerFacts(caller = PersonAccess.OWNER, taken = setOf("dan")),
    ): CeremonyStore {
        val store = store(core, directory, facts)
        store.read(offer("user_bob"), listOf(buildBox, gpu), null)
        assertEquals("nothing about another account's code is shown before the check", CeremonyStore.Phase.CheckingTeam, store.phase.value)
        assertNull(store.offer.value)
        store.checkTeammate()
        return store
    }

    private val gate: suspend (String, String) -> ConfirmingTap.Outcome = { _, _ -> ConfirmingTap.Outcome.CONFIRMED }

    @Test
    fun aTeammatesCodeOpensTheirConfirmationWithTheDirectorysEmailAndNothingTicked() = runTest {
        val store = scanned()
        assertEquals(CeremonyStore.Phase.Confirming, store.phase.value)
        val teammate = store.teammate.value!!
        assertEquals(CeremonyTeammate("user_bob", "bob.smith@example.com", "Platform"), teammate.who)
        assertEquals(Triple("bob-smith", "Bob Smith", TeammateScope.CONTROL), Triple(teammate.handle, teammate.name, teammate.scope))
        assertEquals("alice@example.com", teammate.grantedBy)
        assertTrue("granted to someone else by default", store.rows.value.none { it.picked })
    }

    @Test
    fun aRunnerThePhoneOnlyControlsIsListedWithWhyAndCantBeTicked() = runTest {
        val store = scanned()
        val blocked = store.rows.value.single { it.runner.id == gpu.id }
        assertEquals(
            "Only an Owner or an Admin can add people to gpu-workstation.",
            blocked.blocked,
        )
        store.toggle(gpu.id)
        store.toggle(buildBox.id)
        assertEquals(listOf("r1"), store.rows.value.filter { it.picked }.map { it.runner.id })
    }

    @Test
    fun anUnreachableDirectoryRefusesAndWritesNothing() = runTest {
        val recorder = Recorder()
        val store = scanned(directory = null)
        assertEquals(CeremonyStore.Phase.Refused(Refusal.DirectoryUnreachable), store.phase.value)
        store.confirmTeammate(gate, recorder.enroller)
        assertTrue(recorder.asked.isEmpty())
    }

    @Test
    fun anAccountOnNoneOfYourTeamsIsRefusedWithTheWayIn() = runTest {
        val core = Core().also { it.scanTeammateAnswer = CeremonyCore.Answer.Refused(Refusal.NotATeammate("")) }
        val store = scanned(core)
        assertEquals(CeremonyStore.Phase.Refused(Refusal.NotATeammate("alice@example.com")), store.phase.value)
        assertTrue((store.phase.value as CeremonyStore.Phase.Refused).refusal.message.contains("sign in to alice@example.com"))
    }

    @Test
    fun aTeammatesMacAskingForAShellAndAHeldDeviceAreRefused() = runTest {
        for (refusal in listOf(Refusal.TeammateShell, Refusal.HeldKey, Refusal.UnregisteredKey)) {
            val store = scanned(Core().also { it.scanTeammateAnswer = CeremonyCore.Answer.Refused(refusal) })
            assertEquals(CeremonyStore.Phase.Refused(refusal), store.phase.value)
        }
    }

    @Test
    fun aPhoneSignedInToNothingStaysOnTheOldMismatchScreen() = runTest {
        val store = CeremonyStore(account = "", accountEmail = "", deviceName = "Alice’s Pixel")
        store.core = Core()
        store.clock = { 1_000L }
        store.read(offer("user_bob"), listOf(buildBox), null)
        assertEquals(CeremonyStore.Phase.Mismatch, store.phase.value)
    }

    @Test
    fun anOwnDevicesCodeIsUnchanged() = runTest {
        val core = Core()
        val store = store(core)
        store.read(offer("user_alice"), listOf(buildBox, gpu), buildBox)
        assertEquals(CeremonyStore.Phase.Confirming, store.phase.value)
        assertNull(store.teammate.value)
        assertEquals(listOf("r1"), store.rows.value.filter { it.picked }.map { it.runner.id })
        assertTrue(core.directories.isEmpty())
    }

    @Test
    fun addTeammateWritesAtTheScopeChosenToTheTickedRunnerOnlyAndAsksNoOneToVouch() = runTest {
        val core = Core()
        val recorder = Recorder()
        val store = scanned(core)
        store.toggle(buildBox.id)
        store.setTeammate { it.copy(access = PersonAccess.VIEWER, handle = "bobby") }

        store.confirmTeammate(gate, recorder.enroller)

        assertEquals(CeremonyStore.Phase.ShowingManifest, store.phase.value)
        val asked = recorder.asked.single()
        assertEquals(TeammateScope.READ, asked.teammate.scope)
        assertEquals("bobby", asked.teammate.handle)
        assertEquals(listOf("build-box"), asked.runners)
        assertEquals(keyA, asked.keyA)
        assertEquals("farcooler-0123456789ab", asked.clientId)
        assertEquals("the reply says whose runners these are", listOf("alice@example.com", "alice@example.com"), core.repliesGrantedBy)
        assertNull(store.teammateNote.value)
        assertFalse(store.someRunnersPending.value)
    }

    @Test
    fun theAccessMenuOffersAdminOnlyWhereThePhoneIsTheOwner() = runTest {
        val owner = scanned(facts = TeammateRunnerFacts(caller = PersonAccess.OWNER))
        owner.toggle(buildBox.id)
        assertEquals(
            listOf(PersonAccess.ADMIN, PersonAccess.MEMBER, PersonAccess.VIEWER),
            TeammateAccessRules.choices(owner.rows.value.filter { it.picked }.map { owner.facts.value[it.runner.id]?.caller }),
        )
        val admin = scanned(facts = TeammateRunnerFacts(caller = PersonAccess.ADMIN))
        admin.toggle(buildBox.id)
        assertEquals(
            listOf(PersonAccess.MEMBER, PersonAccess.VIEWER),
            TeammateAccessRules.choices(admin.rows.value.filter { it.picked }.map { admin.facts.value[it.runner.id]?.caller }),
        )
    }

    @Test
    fun thePersonAlreadyUnderTheAccountIsPickedToStartAndNewPersonIsTheOtherChoice() = runTest {
        val bob = ExistingPerson("bob", "Bob", PersonAccess.MEMBER)
        val store = scanned(facts = TeammateRunnerFacts(caller = PersonAccess.OWNER, candidates = listOf(bob), matching = "bob"))
        assertEquals("bob", store.teammate.value!!.person)
        store.setTeammate { it.copy(person = null) }
        assertNull("New person", store.teammate.value!!.person)
    }

    @Test
    fun aPhoneThatSignsAsksOnceForTheRequestAndNotForTheOldTap() = runTest {
        val recorder = Recorder()
        val signing = scanned(facts = TeammateRunnerFacts(caller = PersonAccess.OWNER, signs = true))
        signing.toggle(buildBox.id)
        var asked = 0
        signing.confirmTeammate({ _, _ -> asked += 1; ConfirmingTap.Outcome.CONFIRMED }, recorder.enroller)
        assertEquals("the prompt is the signature's", 0, asked)
        assertEquals(1, recorder.asked.size)

        val plain = scanned(facts = TeammateRunnerFacts(caller = PersonAccess.OWNER, signs = false))
        plain.toggle(buildBox.id)
        plain.confirmTeammate({ _, _ -> asked += 1; ConfirmingTap.Outcome.CONFIRMED }, recorder.enroller)
        assertEquals("a phone that can't sign still confirms", 1, asked)
    }

    @Test
    fun aRunnerThatCouldNotBindIsNamedAndPending() = runTest {
        val store = scanned()
        store.toggle(buildBox.id)
        store.confirmTeammate(gate, Recorder(TeammateAddOutcome.TooOld).enroller)
        assertEquals(CeremonyStore.Phase.ShowingManifest, store.phase.value)
        assertEquals(
            "build-box’s Far Cooler is too old to record whose device this is, so nothing was added there. " +
                "Update Far Cooler on it, then add your teammate again.",
            store.teammateNote.value,
        )
        assertTrue(store.someRunnersPending.value)
    }

    @Test
    fun aCancelledOrDeclinedTapWritesNothing() = runTest {
        for (tap in listOf(ConfirmingTap.Outcome.CANCELLED, ConfirmingTap.Outcome.REFUSED)) {
            val recorder = Recorder()
            val store = scanned()
            store.toggle(buildBox.id)
            store.confirmTeammate({ _, _ -> tap }, recorder.enroller)
            assertTrue("$tap wrote", recorder.asked.isEmpty())
            assertEquals(CeremonyStore.Phase.Confirming, store.phase.value)
            assertEquals(tap == ConfirmingTap.Outcome.REFUSED, store.declined.value)
        }
    }

    @Test
    fun aHandleARunnerWouldRefuseWritesNothing() = runTest {
        val recorder = Recorder()
        val store = scanned()
        store.toggle(buildBox.id)
        store.setTeammate { it.copy(handle = "Bob Smith") }
        var asked = false
        store.confirmTeammate({ _, _ -> asked = true; ConfirmingTap.Outcome.CONFIRMED }, recorder.enroller)
        assertFalse("nothing is asked of the fingerprint either", asked)
        assertTrue(recorder.asked.isEmpty())
    }

    @Test
    fun aReplyThatCantBeBuiltIsRefusedBeforeAnyKeyIsWritten() = runTest {
        val recorder = Recorder()
        val core = Core().also { it.replyAnswer = CeremonyCore.Answer.Refused(Refusal.TooLarge) }
        val store = scanned(core)
        store.toggle(buildBox.id)
        store.confirmTeammate(gate, recorder.enroller)
        assertEquals(CeremonyStore.Phase.Refused(Refusal.TooLarge), store.phase.value)
        assertTrue(recorder.asked.isEmpty())
    }

    @Test
    fun aSlowCheckForAnEarlierCodeNeverOpensItsSheetOverALaterOne() = runTest {
        val store = store(Core())
        val answer: String? = "{}"
        val gateOpen = kotlinx.coroutines.CompletableDeferred<Unit>()
        store.teamDirectory = { gateOpen.await(); answer }
        store.read(offer("user_bob"), listOf(buildBox), null)
        val early = launch(UnconfinedTestDispatcher(testScheduler)) { store.checkTeammate() }
        // A second code arrives while the directory is still answering.
        store.read(offer("user_alice"), listOf(buildBox), buildBox)
        gateOpen.complete(Unit)
        early.join()
        assertNull("the earlier check opened a teammate's sheet", store.teammate.value)
        assertEquals(CeremonyStore.Phase.Confirming, store.phase.value)
    }
}
