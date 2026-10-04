package com.farcooler

import com.farcooler.account.Account
import com.farcooler.notify.Notifier
import com.farcooler.notify.PushMessage
import com.farcooler.notify.TaskNotice
import com.farcooler.notify.TaskNotices
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
            // A producer that rewrote its fixture in CI would pass by definition.
            check(System.getenv("CI") != "true") { "FARCOOLER_WRITE_CONTRACTS is set under CI" }
            contractFile(path).writeText(pretty.encodeToString(JsonElement.serializer(), sent) + "\n")
        }
        assertEquals("$path is not what Account.registration sends", contract(path), sent)
    }

    /** What `onMessageReceived` reads off each FCM fixture. */
    private val expected: Map<String, PushMessage> = mapOf(
        "agent-blocked" to PushMessage.Card(
            title = "claude needs you",
            body = "auth-refactor — Do you want to run git push --force-with-lease?",
            terminal = "term-01999a8f2c4e",
            channel = Notifier.CHANNEL_BLOCKED,
            // The runner rides with a pane too (ov-183): a tap looks there first.
            kind = null, task = null, runner = runner,
        ),
        "agent-done-failed" to PushMessage.Card(
            title = "codex failed",
            body = "pdf-export — Its last turn didn’t finish",
            terminal = "term-01999a90aa10",
            channel = Notifier.CHANNEL_DONE,
            // The runner rides with a pane too (ov-183): a tap looks there first.
            kind = null, task = null, runner = runner,
        ),
        "decision-legacy" to PushMessage.Task(
            TaskNotice("ov-90", runner, "decision", noticeId, listOf("pdfkit", "pdf.js")),
            "ov-90 Pick a PDF library",
            "Needs your decision · Which PDF library should export use?",
        ),
        // A runner older than ov-94 sends no event, so this is no task notice:
        // it reads as a card that carries the task and its runner.
        "decision-old-runner" to PushMessage.Card(
            title = "ov-90 Pick a PDF library",
            body = "Needs your decision · Which PDF library should export use?",
            terminal = "",
            channel = Notifier.CHANNEL_BLOCKED,
            kind = "decision", task = "ov-90", runner = runner,
        ),
        "task-decision" to PushMessage.Task(
            TaskNotice("ov-90", runner, "decision", noticeId, listOf("pdfkit", "pdf.js")),
            "ov-90 Pick a PDF library",
            "Needs your decision · Which PDF library should export use?",
        ),
        "task-review" to PushMessage.Task(
            TaskNotice("ov-90", runner, "review", noticeId, emptyList()),
            "ov-90 Pick a PDF library",
            "Moved to In Review · 3 files changed",
        ),
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
            val read = PushMessage.of(
                data,
                notification?.get("title")?.jsonPrimitive?.content,
                notification?.get("body")?.jsonPrimitive?.content,
            )
            assertEquals(name, expected.getValue(name), read)

            // Where Firebase files a card it draws, and where this app files
            // one it draws itself, must be the same channel.
            val firebase = message["android"]?.jsonObject?.get("notification")?.jsonObject
            if (firebase != null) {
                val channel = firebase.getValue("channel_id").jsonPrimitive.content
                when (read) {
                    is PushMessage.Card -> assertEquals(name, read.channel, channel)
                    is PushMessage.Task -> assertEquals(name, TaskNotices.channelFor(read.notice.event!!), channel)
                    null -> throw AssertionError("$name read as nothing")
                }
            } else {
                assertNull("$name: a card the app draws has no Firebase notification", notification)
            }
        }
    }

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
