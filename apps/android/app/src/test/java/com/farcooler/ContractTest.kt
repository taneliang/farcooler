package com.farcooler

import com.farcooler.account.Account
import com.farcooler.notify.NotificationCopy
import com.farcooler.notify.TaskNotice
import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/**
 * The JSON this app shares with the relay, against the fixtures in
 * `test/fixtures/contracts/` (ov-121): the registration [Account] sends, and
 * the FCM messages the relay sends this app. The relay's suite files the one
 * and writes the other, so a key renamed on either side fails one suite or
 * the other. See the README beside the fixtures.
 */
class ContractTest {
    private val runner = "3a342465-0508-031f-1852-5550524f0f01"
    private val noticeId = "t:$runner:ov-90"

    @Test
    fun `the Android registration fixture is what registerDevice sends`() {
        val sent = Account.registration(
            pushToken = "fP3kQ9xR2sT:APA91bH7mN4vW8yZ1aC5dE9gJ2kL6nP0qS3tU7wX1zB4cF8hK2mO5rV9yA3dG7jL0pS4uW8xZ2bE6",
            platform = "fcm",
            label = "Pixel 9",
            version = "0.2.0 (canary) · 412",
            notifyOnDone = true,
            notifyEvents = listOf("decision", "review", "blocked"),
        )
        val path = "registration/android.json"
        if (System.getenv("FARCOOLER_WRITE_CONTRACTS") != null) {
            contractFile(path).writeText(pretty.encodeToString(JsonElement.serializer(), sent) + "\n")
        }
        assertEquals("$path is not what Account.registration sends", contract(path), sent)
    }

    /** What each FCM fixture is to this app: a task notice, or none. */
    private val expected: Map<String, TaskNotice?> = mapOf(
        "agent-blocked" to null,
        "agent-done-failed" to null,
        "decision-legacy" to TaskNotice("ov-90", runner, "decision", noticeId, listOf("pdfkit", "pdf.js")),
        "task-decision" to TaskNotice("ov-90", runner, "decision", noticeId, listOf("pdfkit", "pdf.js")),
        "task-review" to TaskNotice("ov-90", runner, "review", noticeId, emptyList()),
    )

    @Test
    fun `every FCM message the relay sends reads as what it is about`() {
        val names = contractFile("push/fcm").list { _, name -> name.endsWith(".json") }!!
            .map { it.removeSuffix(".json") }.sorted()
        assertEquals("every FCM fixture has an expectation", expected.keys.sorted(), names)

        for (name in names) {
            val message = contract("push/fcm/$name.json").getValue("message").jsonObject
            // `RemoteMessage.data` is a map of strings, and so is this.
            val data = message.getValue("data").jsonObject.mapValues { it.value.jsonPrimitive.content }
            val notification = message["notification"]?.jsonObject
            assertEquals(name, expected.getValue(name), TaskNotice.of(data))

            // What `onMessageReceived` titles the card with: the data's own
            // title for a card the app draws, else the notification's.
            val title = data["title"] ?: notification?.get("title")?.jsonPrimitive?.content
            assertEquals(name, expectedTitle(name), title)

            // Where Firebase files a card it draws, and where this app files
            // one it draws itself, must be the same channel.
            val firebase = message["android"]?.jsonObject?.get("notification")?.jsonObject
            if (firebase != null) {
                assertEquals(name, NotificationCopy.channelForPush(data), firebase.getValue("channel_id").jsonPrimitive.content)
            } else {
                assertNull("$name: a card the app draws has no Firebase notification", notification)
            }
        }
        assertEquals("term-01999a8f2c4e", fcmData("agent-blocked")["terminal"])
        assertEquals("blocked", fcmData("agent-blocked")["status"])
    }

    private fun expectedTitle(name: String): String = when {
        name.startsWith("agent-blocked") -> "claude needs you"
        name == "agent-done-failed" -> "codex failed"
        else -> "ov-90 Pick a PDF library"
    }

    private fun fcmData(name: String): Map<String, String> =
        contract("push/fcm/$name.json").getValue("message").jsonObject.getValue("data").jsonObject
            .mapValues { it.value.jsonPrimitive.content }

    private val pretty = Json { prettyPrint = true; prettyPrintIndent = "  " }

    private fun contract(path: String): JsonObject =
        Json.parseToJsonElement(contractFile(path).readText()).jsonObject

    /** A file under `test/fixtures/contracts`, found by walking up from wherever Gradle runs. */
    private fun contractFile(path: String): File {
        var directory: File? = File(System.getProperty("user.dir") ?: ".").absoluteFile
        while (directory != null) {
            val root = File(directory, "test/fixtures/contracts")
            if (root.isDirectory) return File(root, path)
            directory = directory.parentFile
        }
        throw AssertionError("Could not find test/fixtures/contracts above ${System.getProperty("user.dir")}.")
    }
}
