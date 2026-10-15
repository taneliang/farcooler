package com.farcooler.account

import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

/**
 * Sign out everywhere (ov-564): what the app sends, how it reads the relay's
 * answer, and that the Settings action asks first. The same rules as AgentKit's.
 */
class SignOutEverywhereTest {
    private fun answered(vararg fields: Pair<String, Boolean>) =
        RelayAnswer.Answered(buildJsonObject { fields.forEach { (k, v) -> put(k, JsonPrimitive(v)) } })

    @Test
    fun theRequestNamesThisInstallsPushToken() {
        assertEquals("abc", Account.signOutEverywhereBody("abc")["pushToken"]?.jsonPrimitive?.content)
        assertEquals(JsonObject(emptyMap()), Account.signOutEverywhereBody(""))
        assertEquals(JsonObject(emptyMap()), Account.signOutEverywhereBody(null))
    }

    @Test
    fun onlyAnExplicitCompleteTrueIsEveryoneEnded() {
        assertEquals(EverywhereOutcome.Complete, Account.everywhereOutcome(answered("ok" to true, "complete" to true)))
        assertEquals(EverywhereOutcome.Partial, Account.everywhereOutcome(answered("ok" to true, "complete" to false)))
        assertEquals(EverywhereOutcome.Partial, Account.everywhereOutcome(answered("ok" to true)))
    }

    @Test
    fun anAnswerThatIsNotAnOkIsAFailureNeverASignOut() {
        assertEquals(EverywhereOutcome.Failed, Account.everywhereOutcome(RelayAnswer.Refused(401, null)))
        assertEquals(EverywhereOutcome.Failed, Account.everywhereOutcome(RelayAnswer.Unreachable("timeout")))
        assertEquals(EverywhereOutcome.Failed, Account.everywhereOutcome(answered("complete" to true)))
    }

    @Test
    fun recentPairingsAreNamedAndACleanSignOutSaysNothing() {
        fun body(vararg labels: String) = buildJsonObject {
            put("runners", kotlinx.serialization.json.JsonArray(labels.map { l -> buildJsonObject { put("label", JsonPrimitive(l)) } }))
        }
        assertEquals(null, Account.afterSignOutEverywhere(EverywhereOutcome.Complete, body()))
        assertEquals(
            "Runners paired recently: Dev Box, CI. Sign in and remove any you don’t recognize.",
            Account.afterSignOutEverywhere(EverywhereOutcome.Complete, body("Dev Box", "CI")),
        )
        val partly = Account.afterSignOutEverywhere(EverywhereOutcome.Partial, body("Dev Box"))!!
        assertTrue(partly.startsWith(Account.SIGN_OUT_EVERYWHERE_PARTLY) && partly.contains("Dev Box"))
        assertEquals(Account.SIGN_OUT_EVERYWHERE_PARTLY, Account.afterSignOutEverywhere(EverywhereOutcome.Partial, null))
    }

    @Test
    fun theCopyIsPlain() {
        assertTrue(Account.SIGN_OUT_EVERYWHERE_PARTLY.contains("sign out everywhere"))
        assertFalse(Account.SIGN_OUT_EVERYWHERE_FAILED.lowercase().contains("error"))
    }

    /** The button raises the dialog; the call lives in the dialog's own confirm button. */
    @Test
    fun theSettingsActionAsksFirst() {
        val source = listOf("src/main/java/com/farcooler/ui/Screens.kt", "app/src/main/java/com/farcooler/ui/Screens.kt")
            .map(::File).first { it.exists() }.readText()
        assertEquals(1, Regex("signOutEverywhere\\(\\)").findAll(source).count())
        assertTrue(source.indexOf("AlertDialog(") in 0 until source.indexOf("model.account.signOutEverywhere()"))
        assertTrue(source.contains("confirmingSignOutEverywhere = true"))
        assertTrue(source.contains("runners keep working. Check them for any you don’t recognize."))
    }
}
